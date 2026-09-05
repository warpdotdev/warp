use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::executor::block_on;
use http::StatusCode;
use instant::Instant;
use uuid::Uuid;
use warp_cli::mcp::MCPSpec;
use warp_core::features::FeatureFlag;
use warp_graphql::mutations::create_managed_mcp_client_config::{
    CreateManagedMcpClientConfigOutput, ManagedMcpTransportKind,
};
use warp_graphql::response_context::ResponseContext;
use warp_managed_secrets::ManagedSecretValue;
use warpui::r#async::{FutureExt as _, Timer};
use warpui::{App, ModelContext, ModelHandle, SingletonEntity as _};

use super::{AgentDriver, AgentDriverError, MANAGED_MCP_RESOLVE_MAX_ATTEMPTS};
use crate::ai::agent_sdk::driver::terminal::TerminalDriver;
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::mcp::builtin::{FACTORY_MCP_INSTALLATION_UUID, FACTORY_MCP_SERVER_NAME};
use crate::ai::mcp::file_based_manager::{FileBasedMCPManager, FileBasedMCPManagerEvent};
use crate::ai::mcp::file_mcp_watcher::PendingScan;
use crate::ai::mcp::parsing::normalize_mcp_json;
use crate::ai::mcp::{
    FileMCPWatcher, FileMCPWatcherEvent, JSONMCPServer, JSONTransportType, MCPProvider,
    MCPServerState, ParsedTemplatableMCPServerResult, TemplatableMCPServerInstallation,
    TemplatableMCPServerManager,
};
use crate::auth::credentials::Credentials;
use crate::server::graphql::GraphQLError;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::managed_mcp::MockManagedMcpClient;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::warp_managed_paths_watcher::warp_managed_mcp_config_path;

#[test]
fn test_normalize_single_cli_server() {
    let input = r#"{"command": "npx", "args": ["-y", "mcp-server"]}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should wrap with a generated name
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let parsed = parsed.as_object().unwrap();
    assert_eq!(parsed.len(), 1);
    let (_name, server) = parsed.iter().next().unwrap();
    assert_eq!(server["command"].as_str().unwrap(), "npx");
}

#[test]
fn test_normalize_single_sse_server() {
    let input = r#"{"url": "http://localhost:3000/mcp", "headers": {"API_KEY": "value"}}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should wrap with a generated name
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let parsed = parsed.as_object().unwrap();
    assert_eq!(parsed.len(), 1);
    let (_name, server) = parsed.iter().next().unwrap();
    assert_eq!(server["url"].as_str().unwrap(), "http://localhost:3000/mcp");
}

#[test]
fn test_normalize_already_wrapped_server() {
    let input = r#"{"my-server": {"command": "npx", "args": []}}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should return as-is (no command/url at top level)
    assert_eq!(result, input);
}

#[test]
fn test_normalize_mcp_servers_wrapper() {
    let input = r#"{"mcpServers": {"server-name": {"command": "npx", "args": []}}}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should return as-is (no command/url at top level)
    assert_eq!(result, input);
}

#[test]
fn test_normalize_servers_wrapper() {
    let input = r#"{"servers": {"server-name": {"url": "http://example.com"}}}"#;
    let result = normalize_mcp_json(input).unwrap();

    // Should return as-is (no command/url at top level)
    assert_eq!(result, input);
}

#[test]
fn test_normalize_invalid_json() {
    let input = "not valid json";
    let result = normalize_mcp_json(input);

    assert!(result.is_err());
}

#[test]
fn test_normalize_cli_server_with_env() {
    let input = r#"{"command": "npx", "args": ["-y", "mcp-server"], "env": {"API_KEY": "secret"}}"#;
    let result = normalize_mcp_json(input).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let parsed = parsed.as_object().unwrap();
    assert_eq!(parsed.len(), 1);
    let (_name, server) = parsed.iter().next().unwrap();
    assert_eq!(server["env"]["API_KEY"].as_str().unwrap(), "secret");
}

#[test]
fn test_normalize_sse_server_with_headers() {
    let input =
        r#"{"url": "http://localhost:5000/mcp", "headers": {"Authorization": "Bearer token"}}"#;
    let result = normalize_mcp_json(input).unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let parsed = parsed.as_object().unwrap();
    assert_eq!(parsed.len(), 1);
    let (_name, server) = parsed.iter().next().unwrap();
    assert_eq!(
        server["headers"]["Authorization"].as_str().unwrap(),
        "Bearer token"
    );
}

fn managed_client_config_output(mcp_config_json: &str) -> CreateManagedMcpClientConfigOutput {
    CreateManagedMcpClientConfigOutput {
        transport_kind: ManagedMcpTransportKind::Command,
        mcp_config_json: mcp_config_json.to_string(),
        proxy_url: None,
        proxy_token: None,
        authorization_header_name: None,
        authorization_header_value: None,
        expires_at: None,
        response_context: ResponseContext {
            server_version: None,
        },
    }
}

fn raw_secret(value: &str) -> ManagedSecretValue {
    ManagedSecretValue::RawValue {
        value: value.to_string(),
    }
}

fn render_installations(
    installations: Vec<TemplatableMCPServerInstallation>,
    secrets: HashMap<String, ManagedSecretValue>,
) -> HashMap<String, JSONMCPServer> {
    AgentDriver::mcp_installations_to_json(installations, &secrets).unwrap()
}

#[test]
fn managed_resolver_local_uuid_does_not_call_managed_client() {
    let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let mock = MockManagedMcpClient::new();
    let local_installed_uuids = HashSet::from([uuid]);

    let resolved = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::Uuid(uuid)],
        &local_installed_uuids,
        Arc::new(mock),
        None,
    ))
    .unwrap();

    assert_eq!(resolved.local_uuids, vec![uuid]);
    assert!(resolved.ephemeral_installations.is_empty());
}

#[test]
fn managed_resolver_non_local_uuid_calls_managed_client() {
    let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let config_json =
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","env":{"API_TOKEN":"{{API_TOKEN}}"}}}}"#;
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(1)
        .returning(move |requested_uid| {
            assert_eq!(requested_uid, uuid.to_string());
            Ok(managed_client_config_output(config_json))
        });

    let resolved = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::Uuid(uuid)],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap();

    assert!(resolved.local_uuids.is_empty());
    assert_eq!(resolved.ephemeral_installations.len(), 1);
}

#[test]
fn well_known_spec_resolves_via_managed_client() {
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(true);
    let config_json = r#"{"mcpServers":{"linear":{"url":"https://app.warp.dev/mcp/integration-proxy/linear","headers":{"Authorization":"Bearer tok"}}}}"#;
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(1)
        .returning(move |requested_uid| {
            assert_eq!(requested_uid, "linear");
            Ok(managed_client_config_output(config_json))
        });

    let resolved = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::WellKnown("linear".to_string())],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap();

    assert!(resolved.local_uuids.is_empty());
    assert_eq!(resolved.ephemeral_installations.len(), 1);
}

#[test]
fn well_known_resolution_failure_skips_server() {
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(true);
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(1)
        .returning(|_| Err(anyhow::anyhow!("Linear is not connected for this team")));

    // Well-known references are server-injected and best-effort: a failure must
    // skip the server, not fail run setup.
    let resolved = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::WellKnown("linear".to_string())],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap();

    assert!(resolved.local_uuids.is_empty());
    assert!(resolved.ephemeral_installations.is_empty());
}

#[test]
fn well_known_resolution_failure_does_not_drop_other_specs() {
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(true);
    let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let config_json =
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","env":{"API_TOKEN":"{{API_TOKEN}}"}}}}"#;
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(2)
        .returning(move |requested_uid| {
            if requested_uid == "linear" {
                Err(anyhow::anyhow!("Linear is not connected for this team"))
            } else {
                assert_eq!(requested_uid, uuid.to_string());
                Ok(managed_client_config_output(config_json))
            }
        });

    let resolved = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[
            MCPSpec::WellKnown("linear".to_string()),
            MCPSpec::Uuid(uuid),
        ],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap();

    assert_eq!(resolved.ephemeral_installations.len(), 1);
}

#[test]
fn well_known_spec_is_skipped_when_flag_disabled() {
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(false);
    // The managed client must not be called for well-known specs when the
    // feature is disabled (e.g. a persisted config from a dogfood build).
    let mock = MockManagedMcpClient::new();

    let resolved = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::WellKnown("linear".to_string())],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap();

    assert!(resolved.local_uuids.is_empty());
    assert!(resolved.ephemeral_installations.is_empty());
}

#[test]
fn managed_command_config_env_placeholder_uses_local_secret() {
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","env":{"API_TOKEN":"{{API_TOKEN}}"}}}}"#,
        None,
        "github",
    )
    .unwrap();
    let rendered = render_installations(
        installations,
        HashMap::from([("API_TOKEN".to_string(), raw_secret("real"))]),
    );

    match &rendered["GitHub MCP"].transport_type {
        JSONTransportType::CLIServer { env, .. } => {
            assert_eq!(env.get("API_TOKEN").map(String::as_str), Some("real"));
        }
        other => panic!("expected CLI server, got {other:?}"),
    }
}

#[test]
fn managed_command_config_arg_placeholder_uses_local_secret() {
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","args":["--token={{API_TOKEN}}"]}}}"#,
        None,
        "github",
    )
    .unwrap();
    let rendered = render_installations(
        installations,
        HashMap::from([("API_TOKEN".to_string(), raw_secret("real"))]),
    );

    match &rendered["GitHub MCP"].transport_type {
        JSONTransportType::CLIServer { args, .. } => {
            assert_eq!(args, &vec!["--token=real".to_string()]);
        }
        other => panic!("expected CLI server, got {other:?}"),
    }
}

#[test]
fn managed_command_config_preserves_literal_env_when_synthesizing_arg_placeholder() {
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","args":["--token={{API_TOKEN}}"],"env":{"LOG_LEVEL":"info"}}}}"#,
        None,
        "github",
    )
    .unwrap();
    let rendered = render_installations(
        installations,
        HashMap::from([("API_TOKEN".to_string(), raw_secret("real"))]),
    );

    match &rendered["GitHub MCP"].transport_type {
        JSONTransportType::CLIServer { args, env, .. } => {
            assert_eq!(args, &vec!["--token=real".to_string()]);
            assert_eq!(env.get("LOG_LEVEL").map(String::as_str), Some("info"));
        }
        other => panic!("expected CLI server, got {other:?}"),
    }
}

#[test]
fn managed_url_config_preserves_proxy_url_and_header() {
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"url":"https://proxy.example/mcp","headers":{"Authorization":"Bearer proxy-token"}}}}"#,
        None,
        "github",
    )
    .unwrap();
    let rendered = render_installations(installations, HashMap::new());

    match &rendered["GitHub MCP"].transport_type {
        JSONTransportType::SSEServer { url, headers } => {
            assert_eq!(url, "https://proxy.example/mcp");
            assert_eq!(
                headers.get("Authorization").map(String::as_str),
                Some("Bearer proxy-token")
            );
        }
        other => panic!("expected SSE server, got {other:?}"),
    }
}

#[test]
fn managed_url_config_preserves_header_despite_colliding_local_secret() {
    // A server-rendered proxy header must not be overwritten by a local secret that
    // happens to share the header's key name (`apply_secrets` implicit key-name match).
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"url":"https://proxy.example/mcp","headers":{"Authorization":"Bearer proxy-token"}}}}"#,
        None,
        "github",
    )
    .unwrap();
    let rendered = render_installations(
        installations,
        HashMap::from([("Authorization".to_string(), raw_secret("local-secret"))]),
    );

    match &rendered["GitHub MCP"].transport_type {
        JSONTransportType::SSEServer { url, headers } => {
            assert_eq!(url, "https://proxy.example/mcp");
            assert_eq!(
                headers.get("Authorization").map(String::as_str),
                Some("Bearer proxy-token")
            );
        }
        other => panic!("expected SSE server, got {other:?}"),
    }
}

#[test]
fn managed_command_config_preserves_literal_env_despite_colliding_local_secret() {
    // A literal env value rendered by the server must survive even when a local secret
    // shares the env key name.
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","env":{"LOG_LEVEL":"info"}}}}"#,
        None,
        "github",
    )
    .unwrap();
    let rendered = render_installations(
        installations,
        HashMap::from([("LOG_LEVEL".to_string(), raw_secret("debug"))]),
    );

    match &rendered["GitHub MCP"].transport_type {
        JSONTransportType::CLIServer { env, .. } => {
            assert_eq!(env.get("LOG_LEVEL").map(String::as_str), Some("info"));
        }
        other => panic!("expected CLI server, got {other:?}"),
    }
}

#[test]
fn managed_command_config_missing_secret_is_rejected_before_serialization() {
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","args":["--token={{API_TOKEN}}"]}}}"#,
        None,
        "github",
    )
    .expect("managed MCP config should parse");
    let error = AgentDriver::mcp_installations_to_json(installations, &HashMap::new())
        .expect_err("an unresolved secret must not reach the harness config");

    match error {
        AgentDriverError::MCPUnresolvedSecrets {
            server_name,
            secret_names,
        } => {
            assert_eq!(server_name, "GitHub MCP");
            assert_eq!(secret_names, vec!["API_TOKEN".to_string()]);
        }
        other => panic!("expected unresolved MCP secrets, got {other:?}"),
    }
}

#[test]
fn ephemeral_mcp_missing_secret_is_filtered_before_spawn() {
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"url":"https://example.com/mcp","headers":{"Authorization":"Bearer {{API_TOKEN}}"}}}}"#,
        None,
        "github",
    )
    .expect("managed MCP config should parse");
    let (ready, failures) =
        AgentDriver::apply_secrets_to_ephemeral_mcp_installations(installations, &HashMap::new());

    assert!(ready.is_empty());
    assert_eq!(
        failures,
        vec!["'GitHub MCP' was not started: unresolved secret reference(s): API_TOKEN".to_string()]
    );
}

// ── Ephemeral MCP installation ids: stable across rebuilds ─────────────────

#[test]
fn ephemeral_installation_id_is_stable_across_resolutions_for_same_run() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440010".parse().unwrap();
    let config_json =
        r#"{"mcpServers":{"slack":{"url":"https://app.warp.dev/mcp/integration-proxy/slack"}}}"#;

    // Same run re-resolving the same server after a rebuild.
    let first = AgentDriver::installations_from_managed_client_config_json(
        config_json,
        Some(task_id),
        "slack",
    )
    .unwrap();
    let second = AgentDriver::installations_from_managed_client_config_json(
        config_json,
        Some(task_id),
        "slack",
    )
    .unwrap();

    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!(
        first[0].uuid(),
        second[0].uuid(),
        "same run + same server must yield the same id across rebuilds"
    );
}

#[test]
fn ephemeral_installation_id_differs_across_runs() {
    let task_id_a: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440011".parse().unwrap();
    let task_id_b: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440012".parse().unwrap();
    let config_json =
        r#"{"mcpServers":{"slack":{"url":"https://app.warp.dev/mcp/integration-proxy/slack"}}}"#;

    let a = AgentDriver::installations_from_managed_client_config_json(
        config_json,
        Some(task_id_a),
        "slack",
    )
    .unwrap();
    let b = AgentDriver::installations_from_managed_client_config_json(
        config_json,
        Some(task_id_b),
        "slack",
    )
    .unwrap();

    assert_ne!(
        a[0].uuid(),
        b[0].uuid(),
        "different runs must not collide onto the same id"
    );
}

#[test]
fn ephemeral_installation_id_differs_across_servers_in_same_run() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440013".parse().unwrap();
    let slack_config =
        r#"{"mcpServers":{"slack":{"url":"https://app.warp.dev/mcp/integration-proxy/slack"}}}"#;
    let linear_config =
        r#"{"mcpServers":{"linear":{"url":"https://app.warp.dev/mcp/integration-proxy/linear"}}}"#;

    let slack = AgentDriver::installations_from_managed_client_config_json(
        slack_config,
        Some(task_id),
        "slack",
    )
    .unwrap();
    let linear = AgentDriver::installations_from_managed_client_config_json(
        linear_config,
        Some(task_id),
        "linear",
    )
    .unwrap();

    assert_ne!(
        slack[0].uuid(),
        linear[0].uuid(),
        "different servers in one run must not collide onto the same id"
    );
}

#[test]
fn ephemeral_installation_id_is_random_without_task_id() {
    let config_json =
        r#"{"mcpServers":{"slack":{"url":"https://app.warp.dev/mcp/integration-proxy/slack"}}}"#;

    let first =
        AgentDriver::installations_from_managed_client_config_json(config_json, None, "slack")
            .unwrap();
    let second =
        AgentDriver::installations_from_managed_client_config_json(config_json, None, "slack")
            .unwrap();

    assert_ne!(
        first[0].uuid(),
        second[0].uuid(),
        "no task_id means no rebuild to survive, so ids stay random"
    );
}

// ── Built-in Factory MCP injection tests ────────────────────────────────────

fn api_key_credentials() -> Credentials {
    Credentials::ApiKey {
        key: "wk-test-key".to_string(),
        owner_type: None,
    }
}

#[test]
fn builtin_factory_mcp_attaches_with_api_key_credentials() {
    let _flag = FeatureFlag::FactoryMcp.override_enabled(true);

    let installation =
        AgentDriver::builtin_factory_mcp_for_run(Some(&api_key_credentials()), &HashSet::new())
            .expect("built-in Factory MCP should attach when eligible");

    assert_eq!(installation.uuid(), FACTORY_MCP_INSTALLATION_UUID);
    assert_eq!(
        installation.templatable_mcp_server().name,
        FACTORY_MCP_SERVER_NAME
    );
}

#[test]
fn builtin_factory_mcp_skipped_when_flag_disabled() {
    let _flag = FeatureFlag::FactoryMcp.override_enabled(false);

    assert!(
        AgentDriver::builtin_factory_mcp_for_run(Some(&api_key_credentials()), &HashSet::new())
            .is_none()
    );
}

#[test]
fn builtin_factory_mcp_skipped_without_credentials() {
    let _flag = FeatureFlag::FactoryMcp.override_enabled(true);

    assert!(AgentDriver::builtin_factory_mcp_for_run(None, &HashSet::new()).is_none());
}

#[test]
fn builtin_factory_mcp_skipped_on_name_collision() {
    let _flag = FeatureFlag::FactoryMcp.override_enabled(true);
    // A user-configured server named `warp-factory` wins over the built-in.
    let taken_server_names = HashSet::from([FACTORY_MCP_SERVER_NAME.to_string()]);

    assert!(
        AgentDriver::builtin_factory_mcp_for_run(Some(&api_key_credentials()), &taken_server_names)
            .is_none()
    );
}

#[test]
fn managed_resolution_failure_includes_uid_and_message() {
    let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(1)
        .returning(|_| Err(anyhow::anyhow!("not active")));

    let err = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::Uuid(uuid)],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap_err();

    match err {
        AgentDriverError::ManagedMcpResolutionFailed { uid, message } => {
            assert_eq!(uid, uuid);
            assert!(message.contains("not active"));
        }
        other => panic!("expected managed MCP resolution failure, got {other:?}"),
    }
}

fn transient_managed_mcp_error() -> anyhow::Error {
    // A transport-level 503 with no GraphQL user-facing payload, matching what
    // `send_graphql_request` produces for a genuinely transient backend failure (as opposed
    // to a `UserFacingError`, which `ManagedMcpClient::create_managed_mcp_client_config`
    // converts into a plain, untyped `anyhow!(message)`).
    anyhow::Error::new(GraphQLError::HttpError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        body: "unavailable".to_string(),
    })
}

#[test]
fn managed_resolution_retries_transient_error_then_succeeds() {
    let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let config_json =
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","env":{"API_TOKEN":"{{API_TOKEN}}"}}}}"#;
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = Arc::clone(&calls);
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(2)
        .returning(move |_| {
            if calls_clone.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(transient_managed_mcp_error())
            } else {
                Ok(managed_client_config_output(config_json))
            }
        });

    let resolved = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::Uuid(uuid)],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap();

    assert_eq!(resolved.ephemeral_installations.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn managed_resolution_does_not_retry_permanent_typed_http_error() {
    // A typed but permanent (403) transport error must fail fast, same as the untyped
    // user-facing error covered by `managed_resolution_failure_includes_uid_and_message`.
    let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(1)
        .returning(|_| {
            Err(anyhow::Error::new(GraphQLError::HttpError {
                status: StatusCode::FORBIDDEN,
                body: "forbidden".to_string(),
            }))
        });

    let err = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::Uuid(uuid)],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap_err();

    assert!(matches!(
        err,
        AgentDriverError::ManagedMcpResolutionFailed { uid, .. } if uid == uuid
    ));
}

#[test]
fn managed_resolution_exhausts_retry_budget_on_persistent_transient_error() {
    let uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = Arc::clone(&calls);
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(MANAGED_MCP_RESOLVE_MAX_ATTEMPTS)
        .returning(move |_| {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            Err(transient_managed_mcp_error())
        });

    let err = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::Uuid(uuid)],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap_err();

    assert!(matches!(
        err,
        AgentDriverError::ManagedMcpResolutionFailed { uid, .. } if uid == uuid
    ));
    // A persistently transient error must still exhaust the full retry budget rather than
    // giving up early, and must not retry beyond it.
    assert_eq!(
        calls.load(Ordering::SeqCst),
        MANAGED_MCP_RESOLVE_MAX_ATTEMPTS
    );
}

#[test]
fn well_known_resolution_retries_transient_error_then_succeeds() {
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(true);
    let config_json = r#"{"mcpServers":{"linear":{"url":"https://app.warp.dev/mcp/integration-proxy/linear","headers":{"Authorization":"Bearer tok"}}}}"#;
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_clone = Arc::clone(&calls);
    let mut mock = MockManagedMcpClient::new();
    mock.expect_create_managed_mcp_client_config()
        .times(2)
        .returning(move |_| {
            if calls_clone.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(transient_managed_mcp_error())
            } else {
                Ok(managed_client_config_output(config_json))
            }
        });

    // A transient failure must not silently skip the server the way a permanent one does
    // (see `well_known_resolution_failure_skips_server`): it should retry and still resolve.
    let resolved = block_on(AgentDriver::resolve_mcp_specs_with_local_uuids(
        &[MCPSpec::WellKnown("linear".to_string())],
        &HashSet::new(),
        Arc::new(mock),
        None,
    ))
    .unwrap();

    assert_eq!(resolved.ephemeral_installations.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

// ── First-turn readiness for file-based MCP servers ─────────────────────────

/// The file-based MCP singletons omitted by the terminal test setup. The watcher is inert, so
/// the manager only ever sees the events a test emits through [`Self::emit`] and no background
/// scan of the real global config paths can race with them.
struct FileBasedMcpFixture {
    watcher: ModelHandle<FileMCPWatcher>,
    manager: ModelHandle<FileBasedMCPManager>,
}

impl FileBasedMcpFixture {
    fn register(app: &mut App) -> Self {
        let watcher = app.add_singleton_model(|_| FileMCPWatcher::new_inert());
        let manager = app.add_singleton_model(FileBasedMCPManager::new);
        Self { watcher, manager }
    }

    /// Delivers `event` to the manager through its real watcher subscription.
    fn emit(&self, app: &mut App, event: FileMCPWatcherEvent) {
        self.watcher.update(app, |_, ctx| ctx.emit(event));
    }

    fn complete_initial_global_scan(&self, app: &mut App) {
        self.emit(
            app,
            FileMCPWatcherEvent::ScanComplete(PendingScan::InitialGlobal),
        );
    }

    fn remove_config(&self, app: &mut App, server: &SimulatedGlobalServer) {
        self.emit(
            app,
            FileMCPWatcherEvent::ConfigRemoved {
                config_path: server.config_path.clone(),
                root_path: server.root_path.clone(),
                provider: MCPProvider::Warp,
            },
        );
    }
}

/// One auto-started global Warp server, as tracked after a simulated parse of the managed
/// Warp MCP config.
struct SimulatedGlobalServer {
    installation_uuid: Uuid,
    config_path: PathBuf,
    root_path: PathBuf,
}

#[derive(Clone, Copy)]
enum InitialGlobalScan {
    Pending,
    Complete,
}

/// Simulates the global Warp config parsing one server, then leaves the initial global scan
/// in `scan` state. `None` when this platform has no managed Warp MCP config path.
fn simulate_global_warp_server(
    app: &mut App,
    fixture: &FileBasedMcpFixture,
    scan: InitialGlobalScan,
) -> Option<SimulatedGlobalServer> {
    let warp_mcp_config_path = warp_managed_mcp_config_path()?;
    let servers = ParsedTemplatableMCPServerResult::from_user_json(
        r#"{"global-warp": {"command": "npx", "args": ["warp"]}}"#,
    )
    .unwrap_or_default();
    fixture.emit(
        app,
        FileMCPWatcherEvent::ConfigParsed {
            config_path: warp_mcp_config_path.config_path.clone(),
            root_path: warp_mcp_config_path.root_path.clone(),
            provider: MCPProvider::Warp,
            servers,
        },
    );
    match scan {
        InitialGlobalScan::Pending => {}
        InitialGlobalScan::Complete => fixture.complete_initial_global_scan(app),
    }

    let installation_uuid = fixture.manager.read(app, |manager, _| {
        manager
            .global_warp_servers()
            .into_iter()
            .map(|installation| installation.uuid())
            .next()
            .expect("the global Warp server should have been auto-started")
    });
    Some(SimulatedGlobalServer {
        installation_uuid,
        config_path: warp_mcp_config_path.config_path,
        root_path: warp_mcp_config_path.root_path,
    })
}

fn setup_agent_driver(app: &mut App) -> ModelHandle<AgentDriver> {
    let terminal_view = add_window_with_terminal(app, None);
    app.add_model(|ctx| {
        let terminal_driver = TerminalDriver::create_from_existing_view(terminal_view, ctx);
        AgentDriver::new_for_test(std::env::temp_dir(), terminal_driver, ctx)
    })
}

fn noop_setup_events(ctx: &ModelContext<AgentDriver>) -> SetupClientEventReporter {
    SetupClientEventReporter::noop(
        ServerApiProvider::as_ref(ctx).get_ai_client(),
        ctx.background_executor(),
    )
}

#[test]
#[serial_test::serial]
fn initial_global_scan_wait_resolves_after_pending_scan_completes() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let fixture = FileBasedMcpFixture::register(&mut app);
        let Some(server) =
            simulate_global_warp_server(&mut app, &fixture, InitialGlobalScan::Pending)
        else {
            return;
        };
        let driver_handle = setup_agent_driver(&mut app);

        let (scan_tx, mut scan_rx) = oneshot::channel::<Vec<Uuid>>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait =
                driver.wait_for_initial_global_file_based_mcp_scan(Duration::from_secs(5), ctx);
            ctx.spawn(wait, move |_, wait_uuids, _| {
                let _ = scan_tx.send(wait_uuids);
            });
        });

        assert!(
            scan_rx.try_recv().unwrap().is_none(),
            "the scan wait must remain pending before the completion event"
        );

        fixture.complete_initial_global_scan(&mut app);

        let wait_uuids = scan_rx
            .await
            .expect("the scan wait should resolve after the completion event");
        assert_eq!(
            wait_uuids,
            vec![server.installation_uuid],
            "the completion event should provide the frozen wait set"
        );
    });
}

fn assert_initial_global_mcp_readiness_wait_unblocks_on(state: MCPServerState) {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let fixture = FileBasedMcpFixture::register(&mut app);
        let Some(server) =
            simulate_global_warp_server(&mut app, &fixture, InitialGlobalScan::Complete)
        else {
            return;
        };
        let driver_handle = setup_agent_driver(&mut app);

        let (ready_tx, mut ready_rx) = oneshot::channel::<()>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait = driver.wait_for_file_based_mcps_running(
                vec![server.installation_uuid],
                Duration::from_secs(5),
                ctx,
            );
            ctx.spawn(wait, move |_, (), _| {
                let _ = ready_tx.send(());
            });
        });

        assert!(
            ready_rx.try_recv().unwrap().is_none(),
            "must not resolve while the server is still starting"
        );

        TemplatableMCPServerManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.change_server_state(server.installation_uuid, state, ctx);
        });

        ready_rx.await.unwrap_or_else(|_| {
            panic!("readiness wait should resolve once the server reaches {state:?}")
        });
    });
}

#[test]
#[serial_test::serial]
fn initial_global_mcp_readiness_wait_unblocks_on_terminal_states() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);
    assert_initial_global_mcp_readiness_wait_unblocks_on(MCPServerState::Running);
    assert_initial_global_mcp_readiness_wait_unblocks_on(MCPServerState::FailedToStart);
}

/// Scan completion and readiness together must stay within one shared timeout budget: a scan
/// that completes near the deadline must leave the following readiness wait only the small
/// remainder, not a fresh full timeout. Regression test for a doubled-budget bug where each
/// phase got its own full `MCP_SERVER_STARTUP_TIMEOUT`.
#[test]
#[serial_test::serial]
fn initial_global_scan_and_readiness_share_one_bounded_timeout_budget() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let fixture = FileBasedMcpFixture::register(&mut app);
        if simulate_global_warp_server(&mut app, &fixture, InitialGlobalScan::Pending).is_none() {
            return;
        }
        let driver_handle = setup_agent_driver(&mut app);

        // A short stand-in for `MCP_SERVER_STARTUP_TIMEOUT` so the test runs fast. The scan
        // completes deliberately close to this deadline, leaving only a small remainder for
        // the following readiness wait, which nothing ever settles.
        let budget = Duration::from_millis(400);
        let scan_completion_delay = budget - Duration::from_millis(80);
        let (done_tx, done_rx) = oneshot::channel::<()>();
        let start = Instant::now();
        driver_handle.update(&mut app, |driver, ctx| {
            let scan = driver.wait_for_initial_global_file_based_mcp_scan(budget, ctx);
            let setup_events = noop_setup_events(ctx);
            let foreground = ctx.spawner();
            ctx.spawn(
                async move {
                    AgentDriver::await_file_based_mcp_startup(
                        SetupStep::InitialGlobalMcpScan,
                        SetupStep::InitialGlobalMcpReadiness,
                        scan,
                        budget,
                        &setup_events,
                        &foreground,
                    )
                    .await
                    .expect("file-based MCP startup is non-fatal");
                    let _ = done_tx.send(());
                },
                |_, _, _| {},
            );
        });

        Timer::after(scan_completion_delay).await;
        fixture.complete_initial_global_scan(&mut app);

        done_rx
            .await
            .expect("the shared startup wait should resolve");
        let elapsed = start.elapsed();
        assert!(
            elapsed >= budget,
            "the readiness wait must actually run for the remainder of the budget rather than \
             being skipped; took {elapsed:?}"
        );
        assert!(
            elapsed < budget + Duration::from_millis(200),
            "scan completion plus readiness must stay within one shared budget ({budget:?}), \
             not compound into a second full timeout; took {elapsed:?}"
        );
    });
}

/// A global config removed *before* the readiness wait ever subscribes must be treated as
/// already settled: an absent/despawned installation is no longer awaited at all, so the wait
/// resolves immediately instead of subscribing and burning its configured timeout. Regression
/// test: the initial pending-check previously only consulted `TemplatableMCPServerManager`
/// state, which never records that an installation was despawned entirely.
#[test]
#[serial_test::serial]
fn initial_global_mcp_readiness_wait_settles_immediately_when_config_removed_before_subscribing() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let fixture = FileBasedMcpFixture::register(&mut app);
        let Some(server) =
            simulate_global_warp_server(&mut app, &fixture, InitialGlobalScan::Complete)
        else {
            return;
        };

        // Remove the global config before the readiness wait is ever created, as if the file
        // was deleted between the scan completing and this join running.
        fixture.remove_config(&mut app, &server);
        let driver_handle = setup_agent_driver(&mut app);

        // A generous configured timeout the readiness wait should NOT need at all.
        let configured_timeout = Duration::from_secs(5);
        let resolved_immediately = driver_handle
            .update(&mut app, |driver, ctx| {
                driver.wait_for_file_based_mcps_running(
                    vec![server.installation_uuid],
                    configured_timeout,
                    ctx,
                )
            })
            .now_or_never();

        assert!(
            resolved_immediately.is_some(),
            "a despawned installation must be settled up front, not subscribed and awaited"
        );
    });
}

/// A global config removed *after* the readiness wait has already subscribed must settle it via
/// the despawn's `NotRunning` transition, rather than exhausting the full configured timeout.
/// Regression test: the readiness wait previously only treated `Running` and `FailedToStart` as
/// terminal, so a deleted config's `NotRunning` was ignored and the wait hung for the whole
/// budget.
///
/// Drives the real `ConfigRemoved` → `DespawnServers` sequence through `FileBasedMCPManager`
/// rather than injecting the `NotRunning` transition directly. The test harness's
/// `TemplatableMCPServerManager` singleton (registered via `Default`, not the production
/// `::new` constructor, so the suite doesn't need the auth/server-API singletons `::new` wires
/// up) does not itself subscribe to `FileBasedMCPManagerEvent::DespawnServers`, so this test
/// stands in for that one subscription — calling only the same public `shutdown_server` the
/// production subscription calls — to let the rest of the removal sequence run for real.
#[test]
#[serial_test::serial]
fn initial_global_mcp_readiness_wait_settles_on_notrunning_after_subscribing() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let fixture = FileBasedMcpFixture::register(&mut app);
        let Some(server) =
            simulate_global_warp_server(&mut app, &fixture, InitialGlobalScan::Complete)
        else {
            return;
        };

        // Stand in for the `DespawnServers` subscription that the production
        // `TemplatableMCPServerManager::new` constructor wires up (and that this harness's
        // `Default`-constructed singleton skips), so the real `ConfigRemoved` below drives a
        // real `shutdown_server` call instead of a directly-injected state transition.
        TemplatableMCPServerManager::handle(&app).update(&mut app, |_, ctx| {
            ctx.subscribe_to_model(&fixture.manager, |me, _, event, ctx| {
                if let FileBasedMCPManagerEvent::DespawnServers { installation_uuids } = event {
                    for uuid in installation_uuids {
                        me.shutdown_server(*uuid, ctx);
                    }
                }
            });
        });
        let driver_handle = setup_agent_driver(&mut app);

        // A generous configured timeout the readiness wait should NOT need: the removal below
        // must settle it almost immediately instead.
        let configured_timeout = Duration::from_secs(5);
        let (ready_tx, ready_rx) = oneshot::channel::<()>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait = driver.wait_for_file_based_mcps_running(
                vec![server.installation_uuid],
                configured_timeout,
                ctx,
            );
            ctx.spawn(wait, move |_, (), _| {
                let _ = ready_tx.send(());
            });
        });

        // Remove the global config, as if the file were deleted after the scan completed and
        // the readiness wait had already subscribed. This is the real sequence: `ConfigRemoved`
        // → `FileBasedMCPManager::remove_if_orphaned` → `DespawnServers` → (via the stand-in
        // subscription above) `shutdown_server` → `NotRunning`.
        fixture.remove_config(&mut app, &server);

        ready_rx
            .with_timeout(Duration::from_millis(500))
            .await
            .expect("the removal's NotRunning transition should settle the wait promptly")
            .expect("readiness wait task should not have been dropped");
    });
}

/// A wait that times out must tear down its own subscription before resolving: otherwise a
/// later, unrelated terminal-state event for one of its still-pending UUIDs reaches its
/// (still-installed) closure, which calls `unsubscribe_from_model` once its own pending set
/// empties -- and that call removes *every* driver-to-manager subscription, silently killing a
/// different, currently-active wait's subscription too. Regression test: start wait A with a
/// short timeout, let it time out, start wait B for a different UUID, then drive A's UUID to a
/// terminal state (the "late" event) and assert B is unaffected -- it must still resolve once
/// its own UUID later settles.
#[test]
#[serial_test::serial]
fn timed_out_wait_does_not_tear_down_a_later_waits_subscription() {
    let _flag_guard = FeatureFlag::FileBasedMcp.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let fixture = FileBasedMcpFixture::register(&mut app);

        // Scope and auto-start eligibility don't matter here -- only that both installations
        // are tracked (so `is_file_based_mcp_pending` finds their hash) and never reach a
        // terminal state on their own. Nothing touches the filesystem, so this path need not
        // exist.
        let root_path = std::env::temp_dir().join(format!("warp-test-mcp-root-{}", Uuid::new_v4()));
        let config_path = root_path.join(".mcp.json");
        let servers = ParsedTemplatableMCPServerResult::from_user_json(
            r#"{"wait-a": {"command": "npx", "args": ["a"]}, "wait-b": {"command": "npx", "args": ["b"]}}"#,
        )
        .unwrap_or_default();
        fixture.emit(
            &mut app,
            FileMCPWatcherEvent::ConfigParsed {
                config_path,
                root_path,
                provider: MCPProvider::Warp,
                servers,
            },
        );
        let (uuid_a, uuid_b) = fixture.manager.read(&app, |manager, _| {
            let uuid_for = |name: &str| {
                manager
                    .file_based_servers()
                    .into_iter()
                    .find(|installation| installation.templatable_mcp_server().name == name)
                    .map(|installation| installation.uuid())
                    .expect("the server should have been tracked")
            };
            (uuid_for("wait-a"), uuid_for("wait-b"))
        });
        let driver_handle = setup_agent_driver(&mut app);

        // Wait A: a short timeout, so it genuinely times out (nothing ever settles uuid_a
        // during this window) before wait B ever starts.
        let (a_tx, a_rx) = oneshot::channel::<()>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait = driver.wait_for_file_based_mcps_running(
                vec![uuid_a],
                Duration::from_millis(50),
                ctx,
            );
            ctx.spawn(wait, move |_, (), _| {
                let _ = a_tx.send(());
            });
        });
        a_rx.await.expect("wait A should resolve once it times out");

        // Wait B starts only after A has already timed out, for a different UUID.
        let (b_tx, mut b_rx) = oneshot::channel::<()>();
        driver_handle.update(&mut app, |driver, ctx| {
            let wait =
                driver.wait_for_file_based_mcps_running(vec![uuid_b], Duration::from_secs(5), ctx);
            ctx.spawn(wait, move |_, (), _| {
                let _ = b_tx.send(());
            });
        });
        assert!(
            b_rx.try_recv().unwrap().is_none(),
            "wait B must not resolve before its own uuid settles"
        );

        // The "late" terminal event for A's uuid: if A's subscription is still installed here,
        // this reaches its stale closure and (A's pending set only ever held this one uuid)
        // calls `unsubscribe_from_model`, tearing down every driver subscription -- B's too.
        TemplatableMCPServerManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.change_server_state(uuid_a, MCPServerState::Running, ctx);
        });
        // If B's subscription were torn down here, dropping its closure would also drop its
        // captured oneshot sender, which cancels B's internal receiver and resolves B's wait
        // (via the non-fatal `SubscriptionDropped` branch) almost immediately -- *before* B's
        // own uuid ever settles. A plain, non-yielding check right after firing the event above
        // cannot observe this: resolving the cancellation requires the executor to poll B's
        // task at least once. Yield via a real timer first so that poll has a chance to happen,
        // then assert B's channel is still empty -- catching a torn-down subscription regardless
        // of whether it resolves via cancellation or (if B's own uuid happened to match) a
        // spurious success.
        Timer::after(Duration::from_millis(50)).await;
        assert!(
            b_rx.try_recv().unwrap().is_none(),
            "A's late terminal event must not have torn down (and so resolved, however \
             indirectly) B's subscription"
        );

        // B's own uuid settling must still resolve B normally, and must do so promptly via the
        // event -- not via wait B's own multi-second internal timeout, which resolves the
        // future regardless of whether its subscription is still installed and so would mask
        // a torn-down subscription if this wait were unbounded.
        TemplatableMCPServerManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.change_server_state(uuid_b, MCPServerState::Running, ctx);
        });
        b_rx.with_timeout(Duration::from_millis(500))
            .await
            .expect(
                "wait B should resolve promptly via the event, not via its own internal timeout",
            )
            .expect("wait B's oneshot sender should not have been dropped");
    });
}
