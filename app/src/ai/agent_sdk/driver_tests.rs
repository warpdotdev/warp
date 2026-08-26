use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Local;
use cloud_object_models::CodeForge;
use futures::channel::oneshot;
use futures::executor::block_on;
use http::StatusCode;
use repo_metadata::{DirectoryWatcher, RepoMetadataEvent, RepoMetadataModel, RepositoryIdentifier};
use tempfile::TempDir;
use warp_cli::agent::Harness;
use warp_cli::mcp::MCPSpec;
use warp_cli::skill::SkillSpec;
use warp_cli::{
    OZ_CLI_ENV, OZ_HARNESS_ENV, OZ_PARENT_RUN_ID_ENV, OZ_RUN_ID_ENV, SERVER_ROOT_URL_OVERRIDE_ENV,
    SESSION_SHARING_SERVER_URL_OVERRIDE_ENV, WS_SERVER_URL_OVERRIDE_ENV,
};
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warp_graphql::mutations::create_managed_mcp_client_config::{
    CreateManagedMcpClientConfigOutput, ManagedMcpTransportKind,
};
use warp_graphql::response_context::ResponseContext;
use warp_managed_secrets::ManagedSecretValue;
use warp_multi_agent_api::response_event;
use warp_util::standardized_path::StandardizedPath;
use warpui::r#async::Timer;
use warpui::{App, SingletonEntity as _};

use super::{
    AgentDriver, AgentDriverError, AgentRunPrompt, CLIAgentSessionStatus, IdleTimeoutSender,
    LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV, LEGACY_OZ_PARENT_STATE_ROOT_ENV,
    MANAGED_MCP_RESOLVE_MAX_ATTEMPTS, OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV,
    OZ_MESSAGE_LISTENER_STATE_ROOT_ENV, PlatformErrorCode, SDKConversationOutputStatus,
    WARP_MESSAGE_LISTENER_STATE_ROOT_ENV, build_secret_env_vars,
    idle_window_for_cli_session_status, idle_window_for_terminal_status,
    setup_failure_status_update, terminal_status_log_outcome,
};
use crate::ai::agent::conversation::ConversationStatus;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionResult, AIAgentActionResultType, AIAgentInput, AIAgentOutput,
    AIAgentOutputMessage, ArtifactCreatedData, CancellationReason, MessageId, RenderableAIError,
    UploadArtifactResult,
};
use crate::ai::agent_sdk::task_env_vars;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::orchestration_events::{
    OrchestrationEventService, PendingEvent, PendingEventDetail,
};
use crate::ai::blocklist::{
    BlocklistAIHistoryModel, RequestInput, ResponseStream, ResponseStreamId,
};
use crate::ai::cloud_environments::{GithubRepo, SourceRepo};
use crate::ai::llms::LLMId;
use crate::ai::mcp::JSONTransportType;
use crate::ai::mcp::builtin::{FACTORY_MCP_INSTALLATION_UUID, FACTORY_MCP_SERVER_NAME};
use crate::ai::mcp::parsing::normalize_mcp_json;
use crate::ai::skills::SkillManager;
use crate::auth::credentials::Credentials;
use crate::server::graphql::GraphQLError;
use crate::server::server_api::managed_mcp::MockManagedMcpClient;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

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
    installations: Vec<crate::ai::mcp::TemplatableMCPServerInstallation>,
    secrets: HashMap<String, ManagedSecretValue>,
) -> HashMap<String, crate::ai::mcp::JSONMCPServer> {
    AgentDriver::mcp_installations_to_json(installations, &secrets).unwrap()
}

#[test]
fn managed_resolver_local_uuid_does_not_call_managed_client() {
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
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
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
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
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
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
fn managed_command_config_missing_secret_leaves_placeholder() {
    let installations = AgentDriver::installations_from_managed_client_config_json(
        r#"{"mcpServers":{"GitHub MCP":{"command":"npx","args":["--token={{API_TOKEN}}"]}}}"#,
        None,
        "github",
    )
    .unwrap();
    let rendered = render_installations(installations, HashMap::new());

    match &rendered["GitHub MCP"].transport_type {
        JSONTransportType::CLIServer { args, .. } => {
            assert_eq!(args, &vec!["--token={{API_TOKEN}}".to_string()]);
        }
        other => panic!("expected CLI server, got {other:?}"),
    }
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
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
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
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
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
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
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
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
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

// ── IdleTimeoutSender tests ──────────────────────────────────────────────────────

#[test]
fn idle_timeout_sender_send_now_delivers_value() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_now(42);
    assert_eq!(rx.try_recv().unwrap(), Some(42));
}

#[test]
fn idle_timeout_sender_send_now_only_delivers_once() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_now(1);
    idle_timeout.end_run_now(2);
    assert_eq!(rx.try_recv().unwrap(), Some(1));
}

#[test]
fn idle_timeout_sender_send_after_delivers_after_timeout() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_after(Duration::from_millis(50), 99);

    // Not yet delivered.
    assert_eq!(rx.try_recv().unwrap(), None);

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rx.try_recv().unwrap(), Some(99));
}

#[test]
fn idle_timeout_sender_cancel_prevents_delivery() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_after(Duration::from_millis(50), 99);
    idle_timeout.cancel_idle_timeout();

    std::thread::sleep(Duration::from_millis(100));
    // Sender was not consumed, so the channel is still open but empty.
    assert_eq!(rx.try_recv().unwrap(), None);
}

#[test]
fn idle_timeout_sender_cancel_then_send_now_delivers() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.end_run_after(Duration::from_millis(50), 1);
    idle_timeout.cancel_idle_timeout();
    idle_timeout.end_run_now(2);

    assert_eq!(rx.try_recv().unwrap(), Some(2));
}

#[test]
fn idle_timeout_sender_later_send_after_supersedes_earlier() {
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    // First timer: long timeout.
    idle_timeout.end_run_after(Duration::from_secs(10), 1);
    // Second timer: short timeout. The first is implicitly cancelled.
    idle_timeout.end_run_after(Duration::from_millis(50), 2);

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rx.try_recv().unwrap(), Some(2));
}

#[test]
fn idle_timeout_sender_complete_with_optional_idle_none_sends_immediately() {
    // `complete_with_optional_idle(None, value)` routes to `end_run_now` and
    // delivers `value` synchronously.
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.complete_with_optional_idle(None, 7);
    assert_eq!(rx.try_recv().unwrap(), Some(7));
}

#[test]
fn idle_timeout_sender_complete_with_optional_idle_some_defers_then_delivers() {
    // `complete_with_optional_idle(Some(d), value)` routes to `end_run_after`
    // and defers delivery by `d`.
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.complete_with_optional_idle(Some(Duration::from_millis(50)), 7);

    // Not delivered yet.
    assert_eq!(rx.try_recv().unwrap(), None);

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(rx.try_recv().unwrap(), Some(7));
}

#[test]
fn idle_timeout_sender_complete_with_optional_idle_some_then_cancel_invalidates_timer() {
    // Cross-path cancellation: the Stage 2c skip-initial-turn driver path
    // schedules a deferred `Success` via `complete_with_optional_idle(Some(_), _)`
    // *before* the history subscription is wired up; a later
    // `AppendedExchange` in that subscription closure invalidates the timer
    // via `cancel_idle_timeout()`. The shared `Arc<AtomicUsize>` generation
    // counter is what makes that work across the two logical code paths.
    // This test exercises the same sequence in isolation: schedule via the
    // helper, then cancel via the unrelated `cancel_idle_timeout` entry point,
    // and verify the value is never delivered.
    let (tx, mut rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx);
    idle_timeout.complete_with_optional_idle(Some(Duration::from_millis(50)), 7);
    idle_timeout.cancel_idle_timeout();

    std::thread::sleep(Duration::from_millis(100));
    // Sender was never consumed by the cancelled timer, so the channel is
    // still open but empty.
    assert_eq!(rx.try_recv().unwrap(), None);
}

#[test]
fn idle_timeout_sender_on_commit_runs_before_value_is_delivered() {
    // `on_commit` must run synchronously, on whichever thread performs the completion send,
    // strictly before the value is observable via the receiver — for both the immediate
    // (`end_run_now`) and deferred (`end_run_after`) paths. `AgentDriver` relies on this
    // ordering to commit a conversation's "exiting" state before anything can observe the
    // run's completion signal (QUALITY-1801).
    let commit_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let commit_ran_for_hook = Arc::clone(&commit_ran);
    let (tx, rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx).with_on_commit(move || {
        commit_ran_for_hook.store(true, Ordering::SeqCst);
    });
    idle_timeout.end_run_now(1);
    assert!(
        commit_ran.load(Ordering::SeqCst),
        "on_commit must have run by the time end_run_now returns"
    );
    assert_eq!(block_on(rx).unwrap(), 1);

    let commit_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let commit_ran_for_hook = Arc::clone(&commit_ran);
    let (tx, rx) = oneshot::channel::<i32>();
    let idle_timeout = IdleTimeoutSender::new(tx).with_on_commit(move || {
        commit_ran_for_hook.store(true, Ordering::SeqCst);
    });
    idle_timeout.end_run_after(Duration::from_millis(20), 2);
    // Awaited rather than polled: resolution only happens after `on_commit` has already run
    // on the same background thread, so there is nothing to race here.
    assert_eq!(block_on(rx).unwrap(), 2);
    assert!(
        commit_ran.load(Ordering::SeqCst),
        "on_commit must have run before the deferred completion was delivered"
    );
}

// ── Terminal-status idle window routing ──────────────────────────────────────────

fn error_status() -> SDKConversationOutputStatus {
    SDKConversationOutputStatus::Error {
        error: RenderableAIError::InternalWarpError,
    }
}

#[test]
fn terminal_error_defers_by_idle_on_fail() {
    // The agent process is the shared-session sharer, so a failed run must be able to outlive
    // its own failure for the session to stay attachable while the sandbox is retained.
    let window =
        idle_window_for_terminal_status(&error_status(), None, Some(Duration::from_secs(15 * 60)));

    assert_eq!(window, Some(Duration::from_secs(15 * 60)));
}

#[test]
fn terminal_error_exits_immediately_without_idle_on_fail() {
    let window =
        idle_window_for_terminal_status(&error_status(), Some(Duration::from_secs(45 * 60)), None);

    assert_eq!(
        window, None,
        "--idle-on-complete must not act as a fallback for a terminal error"
    );
}

#[test]
fn non_error_completion_defers_by_idle_on_complete() {
    // The failure window must not leak into the success/blocked/cancelled lifecycle.
    let cases = [
        ("success", SDKConversationOutputStatus::Success),
        (
            "blocked",
            SDKConversationOutputStatus::Blocked {
                blocked_action: "approve".to_string(),
            },
        ),
        (
            "cancelled",
            SDKConversationOutputStatus::Cancelled {
                reason: CancellationReason::ManuallyCancelled,
            },
        ),
    ];

    for (label, status) in cases {
        let window = idle_window_for_terminal_status(
            &status,
            Some(Duration::from_secs(45 * 60)),
            Some(Duration::from_secs(15 * 60)),
        );

        assert_eq!(
            window,
            Some(Duration::from_secs(45 * 60)),
            "unexpected window for {label}"
        );
    }
}

#[test]
fn setup_failure_is_reported_as_an_environment_setup_failure() {
    // Not just a label: `TaskStatusMessage::is_environment_setup_failure` matches this variant
    // alone, and the cloud-continuation resolver uses it to decide that a setup failure with no
    // conversation gets a tombstone with no continue CTA. A generic code silently reroutes those
    // runs into continuation handling that has nothing to continue.
    let status = setup_failure_status_update("Environment setup failed: bad command".to_string());

    assert_eq!(
        status.error_code,
        Some(PlatformErrorCode::EnvironmentSetupFailed)
    );
}

#[test]
fn debug_window_refresh_uses_the_most_recently_armed_outcome() {
    // A run can fail, be resumed, and fail again. The refresh subscription is installed once and
    // outlives each individual failure, so refreshing must reschedule the *current* outcome; an
    // outcome captured at first arm would exit the run reporting the earlier failure.
    let (tx, rx) = oneshot::channel::<SDKConversationOutputStatus>();
    let idle_timeout = IdleTimeoutSender::new(tx);

    idle_timeout.arm_refreshable(Duration::from_secs(15 * 60), error_status());
    idle_timeout.arm_refreshable(
        Duration::ZERO,
        SDKConversationOutputStatus::Blocked {
            blocked_action: "second failure".to_string(),
        },
    );

    assert_eq!(
        idle_timeout.refresh(),
        Some(Duration::ZERO),
        "refresh should reschedule using the window most recently armed"
    );

    // Awaited rather than polled: the reschedule completes on a timer task, so a `try_recv` here
    // races it and only passes when the machine is idle.
    let blocked_action = match block_on(rx) {
        Ok(SDKConversationOutputStatus::Blocked { blocked_action }) => Some(blocked_action),
        _ => None,
    };
    assert_eq!(
        blocked_action.as_deref(),
        Some("second failure"),
        "refresh rescheduled a stale outcome instead of the most recent failure"
    );
}

#[test]
fn debug_window_refresh_is_inert_before_anything_is_armed() {
    let (tx, _rx) = oneshot::channel::<SDKConversationOutputStatus>();
    let idle_timeout = IdleTimeoutSender::new(tx);

    assert_eq!(idle_timeout.refresh(), None);
}

#[test]
fn cancelling_the_idle_timeout_stops_a_later_refresh_from_resurrecting_it() {
    // A follow-up cancels the pending exit, but the viewer-input subscription outlives that
    // cancellation. Without clearing the armed outcome, typing in the session afterwards would
    // reschedule the old failure and exit the run mid-follow-up.
    let (tx, mut rx) = oneshot::channel::<SDKConversationOutputStatus>();
    let idle_timeout = IdleTimeoutSender::new(tx);

    // Long enough that the armed timer cannot fire before the cancellation below, which would
    // otherwise make this race under load rather than test the cancellation.
    idle_timeout.arm_refreshable(Duration::from_secs(60), error_status());
    idle_timeout.cancel_idle_timeout();

    assert_eq!(
        idle_timeout.refresh(),
        None,
        "a cancelled debug window must not be refreshable"
    );
    assert!(
        matches!(rx.try_recv(), Ok(None)),
        "the run must not have been ended by the cancelled window"
    );
}

#[test]
fn failed_cli_harness_session_defers_by_idle_on_fail() {
    // The flag lives on `warp agent run`, so it has to behave the same whichever harness the run
    // uses; a failed CLI session is the same "process is the session sharer" situation.
    let idle_on_complete = Some(Duration::from_secs(45 * 60));
    let idle_on_fail = Some(Duration::from_secs(15 * 60));

    let failed = CLIAgentSessionStatus::Failed {
        error_type: None,
        message: Some("boom".to_string()),
    };
    assert_eq!(
        idle_window_for_cli_session_status(&failed, idle_on_complete, idle_on_fail),
        idle_on_fail
    );
    assert_eq!(
        idle_window_for_cli_session_status(&failed, idle_on_complete, None),
        None,
        "--idle-on-complete must not act as a fallback for a failed CLI session"
    );
    assert_eq!(
        idle_window_for_cli_session_status(
            &CLIAgentSessionStatus::Success,
            idle_on_complete,
            idle_on_fail
        ),
        idle_on_complete
    );
    assert_eq!(
        idle_window_for_cli_session_status(
            &CLIAgentSessionStatus::InProgress,
            idle_on_complete,
            idle_on_fail
        ),
        None
    );
    assert_eq!(
        idle_window_for_cli_session_status(
            &CLIAgentSessionStatus::Cancelled,
            idle_on_complete,
            idle_on_fail
        ),
        idle_on_complete,
        "a Ctrl-C cancellation is a non-error completion, like Success or Blocked"
    );
}

#[test]
fn terminal_status_log_outcome_labels_are_low_cardinality() {
    assert_eq!(
        terminal_status_log_outcome(&SDKConversationOutputStatus::Success),
        "non_error_completion"
    );
    assert_eq!(terminal_status_log_outcome(&error_status()), "error");
}

#[test]
fn task_env_vars_include_parent_run_id_when_present() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), Some("parent-run-123"), Harness::Claude);
    let overrides_allowed = ChannelState::channel().allows_server_url_overrides();

    assert_eq!(
        env_vars.get(&OsString::from(OZ_RUN_ID_ENV)),
        Some(&OsString::from(task_id.to_string()))
    );
    assert_eq!(
        env_vars.get(&OsString::from(OZ_PARENT_RUN_ID_ENV)),
        Some(&OsString::from("parent-run-123"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(OZ_HARNESS_ENV)),
        Some(&OsString::from("claude"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV)),
        Some(&OsString::from("1"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(
            LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV
        )),
        Some(&OsString::from("1"))
    );
    assert!(
        env_vars
            .get(&OsString::from(OZ_CLI_ENV))
            .is_some_and(|value| !value.is_empty())
    );

    let server_root_url = ChannelState::server_root_url().into_owned();
    if overrides_allowed && !server_root_url.is_empty() {
        assert_eq!(
            env_vars.get(&OsString::from(SERVER_ROOT_URL_OVERRIDE_ENV)),
            Some(&OsString::from(server_root_url))
        );
    } else {
        assert!(!env_vars.contains_key(&OsString::from(SERVER_ROOT_URL_OVERRIDE_ENV)));
    }

    let ws_server_url = ChannelState::ws_server_url().into_owned();
    if overrides_allowed && !ws_server_url.is_empty() {
        assert_eq!(
            env_vars.get(&OsString::from(WS_SERVER_URL_OVERRIDE_ENV)),
            Some(&OsString::from(ws_server_url))
        );
    } else {
        assert!(!env_vars.contains_key(&OsString::from(WS_SERVER_URL_OVERRIDE_ENV)));
    }

    if overrides_allowed {
        match ChannelState::session_sharing_server_url() {
            Some(url) if !url.is_empty() => assert_eq!(
                env_vars.get(&OsString::from(SESSION_SHARING_SERVER_URL_OVERRIDE_ENV)),
                Some(&OsString::from(url.into_owned()))
            ),
            _ => {
                assert!(
                    !env_vars
                        .contains_key(&OsString::from(SESSION_SHARING_SERVER_URL_OVERRIDE_ENV))
                )
            }
        }
    } else {
        assert!(!env_vars.contains_key(&OsString::from(SESSION_SHARING_SERVER_URL_OVERRIDE_ENV)));
    }
}

#[test]
fn task_env_vars_omit_parent_run_id_when_absent() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440001".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), None, Harness::Oz);
    let overrides_allowed = ChannelState::channel().allows_server_url_overrides();

    assert_eq!(
        env_vars.get(&OsString::from(OZ_RUN_ID_ENV)),
        Some(&OsString::from(task_id.to_string()))
    );
    assert!(!env_vars.contains_key(&OsString::from(OZ_PARENT_RUN_ID_ENV)));
    assert_eq!(
        env_vars.get(&OsString::from(OZ_HARNESS_ENV)),
        Some(&OsString::from("oz"))
    );
    assert!(!env_vars.contains_key(&OsString::from(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV)));
    assert!(!env_vars.contains_key(&OsString::from(
        LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV
    )));
    assert_eq!(
        env_vars.contains_key(&OsString::from(SERVER_ROOT_URL_OVERRIDE_ENV)),
        overrides_allowed && !ChannelState::server_root_url().is_empty()
    );
    assert_eq!(
        env_vars.contains_key(&OsString::from(WS_SERVER_URL_OVERRIDE_ENV)),
        overrides_allowed && !ChannelState::ws_server_url().is_empty()
    );
}

#[test]
fn task_env_vars_enable_external_parent_listener_for_claude_runs_without_parent_run_id() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440002".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), None, Harness::Claude);
    assert_eq!(
        env_vars.get(&OsString::from(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV)),
        Some(&OsString::from("1"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(
            LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV
        )),
        Some(&OsString::from("1"))
    );
}

#[test]
#[serial_test::serial]
fn task_env_vars_propagate_message_listener_state_root_with_legacy_alias() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440003".parse().unwrap();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe {
        std::env::set_var(
            OZ_MESSAGE_LISTENER_STATE_ROOT_ENV,
            "/tmp/message-listener-root",
        )
    };
    let env_vars = task_env_vars(Some(&task_id), None, Harness::Claude);
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV) };

    assert_eq!(
        env_vars.get(&OsString::from(OZ_MESSAGE_LISTENER_STATE_ROOT_ENV)),
        Some(&OsString::from("/tmp/message-listener-root"))
    );
    // The WARP_ twin is only reachable when the state root is actually set, which is why it is
    // asserted here rather than in the pairing test: that test does not populate the process
    // env, and making it do so would need `#[serial_test::serial]` to stay race-free.
    assert_eq!(
        env_vars.get(&OsString::from(WARP_MESSAGE_LISTENER_STATE_ROOT_ENV)),
        Some(&OsString::from("/tmp/message-listener-root"))
    );
    assert_eq!(
        env_vars.get(&OsString::from(LEGACY_OZ_PARENT_STATE_ROOT_ENV)),
        Some(&OsString::from("/tmp/message-listener-root"))
    );
}

/// Every `OZ_` variable reaching a harness subprocess has a `WARP_` twin carrying the same
/// value. This guard fails when one of a pair is injected without the other.
///
/// The legacy `OZ_PARENT_*` listener names are exempt: they exist only to keep an external
/// Claude plugin working through its migration and are deliberately not given `WARP_` twins.
#[test]
fn task_env_vars_mirror_every_oz_var_to_a_warp_name() {
    const LEGACY_ONLY: [&str; 2] = [
        LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV,
        LEGACY_OZ_PARENT_STATE_ROOT_ENV,
    ];

    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440005".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), Some("parent-run-789"), Harness::Claude);

    let mut paired = 0;
    for (name, value) in &env_vars {
        let Some(name) = name.to_str() else { continue };
        let Some(suffix) = name.strip_prefix("OZ_") else {
            continue;
        };
        if LEGACY_ONLY.contains(&name) {
            continue;
        }
        paired += 1;
        let warp_name = OsString::from(format!("WARP_{suffix}"));
        assert_eq!(
            env_vars.get(&warp_name),
            Some(value),
            "{name} is injected without a matching WARP_{suffix} carrying the same value"
        );
    }
    assert!(
        paired > 0,
        "no OZ_ variables were injected, so the assertions above would prove nothing"
    );
}

#[test]
fn task_env_vars_can_use_opencode_harness() {
    let task_id: AmbientAgentTaskId = "550e8400-e29b-41d4-a716-446655440004".parse().unwrap();
    let env_vars = task_env_vars(Some(&task_id), Some("parent-run-456"), Harness::OpenCode);

    assert_eq!(
        env_vars.get(&OsString::from(OZ_HARNESS_ENV)),
        Some(&OsString::from("opencode"))
    );
}

#[test]
fn json_format_output_includes_filename_for_file_artifact_created_event() {
    let output = AIAgentOutput {
        messages: vec![AIAgentOutputMessage::artifact_created(
            MessageId::new("message-1".to_string()),
            ArtifactCreatedData::File {
                artifact_uid: "artifact-uid".to_string(),
                filepath: "outputs/report.txt".to_string(),
                filename: "report.txt".to_string(),
                mime_type: "text/plain".to_string(),
                description: Some("Build output for the latest run".to_string()),
                size_bytes: 42,
            },
        )],
        ..Default::default()
    };

    let mut bytes = Vec::new();
    super::output::json::format_output(&output, &mut bytes).expect("json formatting should work");

    let value: serde_json::Value =
        serde_json::from_slice(&bytes).expect("output should be valid json");

    assert_eq!(value["type"], "artifact_created");
    assert_eq!(value["artifact_type"], "file");
    assert_eq!(value["artifact_uid"], "artifact-uid");
    assert_eq!(value["filepath"], "outputs/report.txt");
    assert_eq!(value["filename"], "report.txt");
    assert_eq!(value["mime_type"], "text/plain");
    assert_eq!(value["description"], "Build output for the latest run");
    assert_eq!(value["size_bytes"], 42);
}

#[test]
fn json_format_input_omits_filepath_and_description_for_proto_upload_result() {
    let input = AIAgentInput::ActionResult {
        result: AIAgentActionResult {
            id: "tool-call-1".to_string().into(),
            task_id: TaskId::new("task-1".to_string()),
            result: AIAgentActionResultType::UploadArtifact(UploadArtifactResult::Success {
                artifact_uid: "artifact-123".to_string(),
                filepath: None,
                mime_type: "text/plain".to_string(),
                description: None,
                size_bytes: 42,
            }),
        },
        context: Arc::from([]),
    };

    let mut bytes = Vec::new();
    super::output::json::format_input(&input, &mut bytes).expect("json formatting should work");

    let value: serde_json::Value =
        serde_json::from_slice(&bytes).expect("output should be valid json");

    assert_eq!(value["type"], "tool_result");
    assert_eq!(value["tool"], "upload_artifact");
    assert_eq!(value["artifact_uid"], "artifact-123");
    assert_eq!(value["mime_type"], "text/plain");
    assert_eq!(value["size_bytes"], 42);
    assert!(value.get("filepath").is_none());
    assert!(value.get("description").is_none());
}

// ── build_secret_env_vars tests ──────────────────────────────────────────────

#[test]
#[serial_test::serial]
fn raw_value_only_writes_under_secret_name() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("MY_SECRET") };
    let secrets = HashMap::from([(
        "MY_SECRET".to_string(),
        ManagedSecretValue::raw_value("s3cret"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("MY_SECRET")),
        Some(&OsString::from("s3cret"))
    );
    assert_eq!(env_vars.len(), 1);
}

#[test]
#[serial_test::serial]
fn anthropic_api_key_writes_anthropic_env_var() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
    let secrets = HashMap::from([(
        "my-custom-name".to_string(),
        ManagedSecretValue::anthropic_api_key("sk-ant-test-key"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("ANTHROPIC_API_KEY")),
        Some(&OsString::from("sk-ant-test-key"))
    );
}

#[test]
#[serial_test::serial]
fn typed_secret_overrides_raw_value_with_same_env_name() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
    let typed_key = "sk-ant-typed-key-abcdef";
    let raw_key = "sk-ant-raw-key-ghijkl";
    let secrets = HashMap::from([
        (
            "my-auth".to_string(),
            ManagedSecretValue::anthropic_api_key(typed_key),
        ),
        (
            "ANTHROPIC_API_KEY".to_string(),
            ManagedSecretValue::raw_value(raw_key),
        ),
    ]);
    // Run multiple times to defeat HashMap iteration order flakiness.
    for _ in 0..20 {
        let env_vars = build_secret_env_vars(&secrets);
        assert_eq!(
            env_vars.get(&OsString::from("ANTHROPIC_API_KEY")),
            Some(&OsString::from(typed_key)),
            "Typed secret must always override RawValue with the same env name"
        );
    }
}

#[test]
#[serial_test::serial]
fn bedrock_api_key_writes_all_bedrock_env_vars() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CODE_USE_BEDROCK") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("AWS_REGION") };
    let secrets = HashMap::from([
        (
            "bedrock-secret".to_string(),
            ManagedSecretValue::anthropic_bedrock_api_key("token-123", "us-west-2"),
        ),
        (
            "AWS_REGION".to_string(),
            ManagedSecretValue::raw_value("eu-west-1"),
        ),
    ]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("AWS_BEARER_TOKEN_BEDROCK")),
        Some(&OsString::from("token-123"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("CLAUDE_CODE_USE_BEDROCK")),
        Some(&OsString::from("1"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("AWS_REGION")),
        Some(&OsString::from("us-west-2")),
        "Typed Bedrock secret should win over RawValue for AWS_REGION"
    );
}

#[test]
#[serial_test::serial]
fn bedrock_access_key_writes_all_aws_env_vars() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("AWS_ACCESS_KEY_ID") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("AWS_SECRET_ACCESS_KEY") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("AWS_SESSION_TOKEN") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CODE_USE_BEDROCK") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("AWS_REGION") };
    let secrets = HashMap::from([(
        "bedrock-access".to_string(),
        ManagedSecretValue::anthropic_bedrock_access_key(
            "AKID",
            "secret-key",
            Some("session-tok".to_string()),
            "ap-southeast-1",
        ),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("AWS_ACCESS_KEY_ID")),
        Some(&OsString::from("AKID"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("AWS_SECRET_ACCESS_KEY")),
        Some(&OsString::from("secret-key"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("AWS_SESSION_TOKEN")),
        Some(&OsString::from("session-tok"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("CLAUDE_CODE_USE_BEDROCK")),
        Some(&OsString::from("1"))
    );
    assert_eq!(
        env_vars.get(&OsString::from("AWS_REGION")),
        Some(&OsString::from("ap-southeast-1"))
    );
}

#[test]
#[serial_test::serial]
fn raw_value_skipped_when_process_env_already_set() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("WORKER_TOKEN", "injected-value") };
    let secrets = HashMap::from([(
        "WORKER_TOKEN".to_string(),
        ManagedSecretValue::raw_value("managed-value"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    // The worker-injected env var wins; env_vars should NOT contain it
    // because the child inherits the process env directly.
    assert!(!env_vars.contains_key(&OsString::from("WORKER_TOKEN")));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("WORKER_TOKEN") };
}

#[test]
#[serial_test::serial]
fn worker_injected_env_wins_over_typed_secret() {
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("ANTHROPIC_API_KEY", "worker-key") };
    let secrets = HashMap::from([(
        "my-auth".to_string(),
        ManagedSecretValue::anthropic_api_key("managed-key"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    // The typed secret should be skipped entirely; the child inherits
    // ANTHROPIC_API_KEY from the process env.
    assert!(!env_vars.contains_key(&OsString::from("ANTHROPIC_API_KEY")));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
}

#[test]
#[serial_test::serial]
fn worker_injected_env_skips_entire_bedrock_secret() {
    // Only AWS_REGION is worker-injected; the entire Bedrock secret should
    // be atomically skipped — no partial insertion.
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var("AWS_REGION", "us-east-1") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("AWS_BEARER_TOKEN_BEDROCK") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("CLAUDE_CODE_USE_BEDROCK") };
    let secrets = HashMap::from([(
        "bedrock-secret".to_string(),
        ManagedSecretValue::anthropic_bedrock_api_key("token-456", "eu-central-1"),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert!(
        !env_vars.contains_key(&OsString::from("AWS_BEARER_TOKEN_BEDROCK")),
        "Entire Bedrock secret must be skipped when any field is worker-injected"
    );
    assert!(!env_vars.contains_key(&OsString::from("CLAUDE_CODE_USE_BEDROCK")));
    assert!(!env_vars.contains_key(&OsString::from("AWS_REGION")));
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("AWS_REGION") };
}

// ── Skill-loading integration test ───────────────────────────────────────────

/// Verifies that `load_environment_skills` loads every skill from an env repo
/// while `load_global_skills` loads only the explicitly requested subset from a
/// global-only repo.
///
/// The test writes real SKILL.md files on disk, seeds `RepoMetadataModel` with a
/// minimal file-tree for the env repo so the indexing wait resolves immediately,
/// and drives both loading methods through a live `AgentDriver` model via a
/// `ModelSpawner`.
#[test]
fn split_loading_env_loads_all_global_loads_subset() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // Create real skill files on disk.
        let temp = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(temp.path()).unwrap();

        // Environment repo: three skills. All should be loaded.
        let env_repo = working_dir.join("env-repo");
        write_skill_file(&env_repo, "build");
        write_skill_file(&env_repo, "test-skill");
        write_skill_file(&env_repo, "deploy");

        // Global-only repo: three skills; only "linter" is explicitly requested.
        let global_repo = working_dir.join("global-repo");
        write_skill_file(&global_repo, "linter");
        write_skill_file(&global_repo, "formatter");
        write_skill_file(&global_repo, "docs");

        // Trigger a real filesystem scan of the env repo so `repository_indexed`
        // resolves immediately once indexing completes.
        let env_repo_std = StandardizedPath::from_local_canonicalized(&env_repo).unwrap();
        let repo_handle = DirectoryWatcher::handle(&app).update(&mut app, |watcher, ctx| {
            watcher.add_directory(env_repo_std.clone(), ctx).unwrap()
        });
        let (indexed_tx, indexed_rx) = futures::channel::oneshot::channel::<()>();
        let tx_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(indexed_tx)));
        let env_repo_for_event = env_repo_std.clone();
        app.update(|ctx| {
            let tx_cell = tx_cell.clone();
            ctx.subscribe_to_model(
                &RepoMetadataModel::handle(ctx),
                move |_, event: &RepoMetadataEvent, _ctx| {
                    if let RepoMetadataEvent::RepositoryUpdated {
                        id: RepositoryIdentifier::Local(path),
                    } = event
                        && *path == env_repo_for_event
                        && let Some(tx) = tx_cell.borrow_mut().take()
                    {
                        let _ = tx.send(());
                    }
                },
            );
        });
        RepoMetadataModel::handle(&app).update(&mut app, |model: &mut RepoMetadataModel, ctx| {
            model.index_directory(repo_handle, ctx).unwrap();
        });
        indexed_rx.await.expect("env repo should be indexed");

        // Construct a minimal AgentDriver backed by a stub terminal view.
        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver_handle = app.add_model(|ctx| {
            let terminal_driver =
                super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
            AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx)
        });

        // Run both loading methods through the driver's ModelSpawner.
        let (done_tx, done_rx) = futures::channel::oneshot::channel::<()>();
        let env_repos = vec![SourceRepo::new(
            CodeForge::GitHub,
            "org".to_string(),
            "env-repo".to_string(),
        )];
        let global_repos = vec![GithubRepo::new(
            "org".to_string(),
            "global-repo".to_string(),
        )];
        let global_specs: Vec<SkillSpec> = ["org/global-repo:linter".to_string()]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        driver_handle.update(&mut app, |_, ctx| {
            let spawner = ctx.spawner();
            ctx.spawn(
                async move {
                    AgentDriver::load_environment_skills(&spawner, env_repos).await;
                    AgentDriver::load_global_skills(&spawner, global_specs, global_repos).await;
                    let _ = done_tx.send(());
                },
                |_, _, _| {},
            );
        });
        done_rx.await.expect("loading task should complete");

        // Verify SkillManager contains the right skills.
        // is_cloud_environment=true (set by both loaders), so get_skills_for_working_directory
        // with cwd=None returns all registered skills.
        let skill_names = SkillManager::handle(&app).read(&app, |manager: &SkillManager, ctx| {
            manager
                .get_skills_for_working_directory(None, ctx)
                .into_iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        });

        assert!(
            skill_names.contains(&"build".to_string()),
            "env skill 'build' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"test-skill".to_string()),
            "env skill 'test-skill' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"deploy".to_string()),
            "env skill 'deploy' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"linter".to_string()),
            "requested global skill 'linter' should be loaded; got: {skill_names:?}"
        );
        assert!(
            !skill_names.contains(&"formatter".to_string()),
            "unrequested global skill 'formatter' should NOT be loaded; got: {skill_names:?}"
        );
        assert!(
            !skill_names.contains(&"docs".to_string()),
            "unrequested global skill 'docs' should NOT be loaded; got: {skill_names:?}"
        );
    });
}

/// Verifies that when a repo is in both the environment list and the global skill
/// specs, all skills from that repo are loaded (environment wins), the targeted
/// global skill is present, and no skill is registered more than once.
#[test]
fn overlap_repo_in_env_and_global_loads_all_skills_without_duplicates() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let temp = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(temp.path()).unwrap();

        // A single repo with three skills, appearing in both the environment
        // and a global spec that targets only one of them.
        let shared_repo = working_dir.join("shared-repo");
        write_skill_file(&shared_repo, "deploy");
        write_skill_file(&shared_repo, "lint");
        write_skill_file(&shared_repo, "test-cmd");

        // Index the repo so `load_environment_skills` can scan it.
        let shared_repo_std = StandardizedPath::from_local_canonicalized(&shared_repo).unwrap();
        let repo_handle = DirectoryWatcher::handle(&app).update(&mut app, |watcher, ctx| {
            watcher.add_directory(shared_repo_std.clone(), ctx).unwrap()
        });
        let (indexed_tx, indexed_rx) = futures::channel::oneshot::channel::<()>();
        let tx_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(indexed_tx)));
        let shared_repo_for_event = shared_repo_std.clone();
        app.update(|ctx| {
            let tx_cell = tx_cell.clone();
            ctx.subscribe_to_model(
                &RepoMetadataModel::handle(ctx),
                move |_, event: &RepoMetadataEvent, _ctx| {
                    if let RepoMetadataEvent::RepositoryUpdated {
                        id: RepositoryIdentifier::Local(path),
                    } = event
                        && *path == shared_repo_for_event
                        && let Some(tx) = tx_cell.borrow_mut().take()
                    {
                        let _ = tx.send(());
                    }
                },
            );
        });
        RepoMetadataModel::handle(&app).update(&mut app, |model: &mut RepoMetadataModel, ctx| {
            model.index_directory(repo_handle, ctx).unwrap();
        });
        indexed_rx.await.expect("shared repo should be indexed");

        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver_handle = app.add_model(|ctx| {
            let terminal_driver =
                super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
            AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx)
        });

        // The same repo is listed in both env repos and global repos.
        // The global spec targets only "deploy".
        let (done_tx, done_rx) = futures::channel::oneshot::channel::<()>();
        let env_repos = vec![SourceRepo::new(
            CodeForge::GitHub,
            "org".to_string(),
            "shared-repo".to_string(),
        )];
        let global_repos = vec![GithubRepo::new(
            "org".to_string(),
            "shared-repo".to_string(),
        )];
        let global_specs: Vec<SkillSpec> = ["org/shared-repo:deploy".to_string()]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        driver_handle.update(&mut app, |_, ctx| {
            let spawner = ctx.spawner();
            ctx.spawn(
                async move {
                    AgentDriver::load_environment_skills(&spawner, env_repos).await;
                    AgentDriver::load_global_skills(&spawner, global_specs, global_repos).await;
                    let _ = done_tx.send(());
                },
                |_, _, _| {},
            );
        });
        done_rx.await.expect("loading task should complete");

        let skill_names = SkillManager::handle(&app).read(&app, |manager: &SkillManager, ctx| {
            manager
                .get_skills_for_working_directory(None, ctx)
                .into_iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        });

        // All three skills from the repo are present (env loading wins).
        assert!(
            skill_names.contains(&"deploy".to_string()),
            "'deploy' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"lint".to_string()),
            "'lint' should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"test-cmd".to_string()),
            "'test-cmd' should be loaded; got: {skill_names:?}"
        );

        // No skill is duplicated.
        let deploy_count = skill_names.iter().filter(|n| *n == "deploy").count();
        assert_eq!(
            deploy_count, 1,
            "'deploy' should appear exactly once; got: {skill_names:?}"
        );
    });
}

/// Write a minimal SKILL.md at `{repo}/.agents/skills/{name}/SKILL.md`.
/// The name is derived from the parent directory name, so no frontmatter is required.
fn write_skill_file(repo: &Path, name: &str) {
    let skill_dir = repo.join(".agents").join("skills").join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), format!("Skill: {name}.")).unwrap();
}

/// Write a minimal SKILL.md at `{skills_dir}/{name}/SKILL.md`.
/// This is the flat layout expected by `WARP_SKILL_DIRS` (no `.agents/skills` wrapper).
fn write_flat_skill(skills_dir: &Path, name: &str) {
    let skill_dir = skills_dir.join(name);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Skill {name}.\n---\n\n# {name}\n"),
    )
    .unwrap();
}

/// Verifies that `load_skills_dirs` reads skills from the `WARP_SKILL_DIRS` environment
/// variable and registers them in the personal (home) bucket so they are always in scope,
/// regardless of the current working directory.
#[test]
#[serial_test::serial]
fn warp_skill_dirs_env_loads_skills_as_home_tier() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let temp = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(temp.path()).unwrap();

        // Create two separate flat skills directories (no .agents/skills prefix).
        let skills_dir_a = working_dir.join("extra-skills-a");
        let skills_dir_b = working_dir.join("extra-skills-b");
        write_flat_skill(&skills_dir_a, "env-skill-a1");
        write_flat_skill(&skills_dir_a, "env-skill-a2");
        write_flat_skill(&skills_dir_b, "env-skill-b1");

        // Point WARP_SKILL_DIRS at both directories.
        let skills_dirs_value = format!("{},{}", skills_dir_a.display(), skills_dir_b.display());
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("WARP_SKILL_DIRS", &skills_dirs_value) };

        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver_handle = app.add_model(|ctx| {
            let terminal_driver =
                super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
            AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx)
        });

        let (done_tx, done_rx) = futures::channel::oneshot::channel::<()>();
        driver_handle.update(&mut app, |_, ctx| {
            let spawner = ctx.spawner();
            ctx.spawn(
                async move {
                    AgentDriver::load_skills_dirs(&spawner).await;
                    let _ = done_tx.send(());
                },
                |_, _, _| {},
            );
        });
        done_rx.await.expect("loading task should complete");

        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("WARP_SKILL_DIRS") };

        // Skills from WARP_SKILL_DIRS are home-tier, so they appear for any working directory.
        // Use None cwd — home skills are included regardless of is_cloud_environment.
        let skill_names = SkillManager::handle(&app).read(&app, |manager: &SkillManager, ctx| {
            manager
                .get_skills_for_working_directory(None, ctx)
                .into_iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        });

        assert!(
            skill_names.contains(&"env-skill-a1".to_string()),
            "'env-skill-a1' from WARP_SKILL_DIRS should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"env-skill-a2".to_string()),
            "'env-skill-a2' from WARP_SKILL_DIRS should be loaded; got: {skill_names:?}"
        );
        assert!(
            skill_names.contains(&"env-skill-b1".to_string()),
            "'env-skill-b1' from WARP_SKILL_DIRS should be loaded; got: {skill_names:?}"
        );

        // Verify the skills have Home scope (personal tier).
        let scope_check = SkillManager::handle(&app).read(&app, |manager: &SkillManager, ctx| {
            use ai::skills::SkillScope;
            manager
                .get_skills_for_working_directory(None, ctx)
                .into_iter()
                .filter(|s| s.name.starts_with("env-skill-"))
                .all(|s| s.scope == SkillScope::Home)
        });
        assert!(
            scope_check,
            "all WARP_SKILL_DIRS skills must have SkillScope::Home"
        );
    });
}

/// Verifies that relative `WARP_SKILL_DIRS` entries are resolved against the driver's
/// working directory rather than the process's current working directory (which
/// `prepare_environment` may have changed).
#[test]
#[serial_test::serial]
fn warp_skill_dirs_env_relative_entries_resolve_against_working_dir() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let temp = TempDir::new().unwrap();
        let working_dir = dunce::canonicalize(temp.path()).unwrap();

        // Create a flat skills directory inside the working dir and reference it by
        // relative path only. No `rel-skills` directory exists under the process cwd,
        // so this only loads if resolution is anchored at the driver's working dir.
        write_flat_skill(&working_dir.join("rel-skills"), "env-skill-rel");

        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("WARP_SKILL_DIRS", "rel-skills") };

        let terminal_view = add_window_with_terminal(&mut app, None);
        let driver_handle = app.add_model(|ctx| {
            let terminal_driver =
                super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
            AgentDriver::new_for_test(working_dir.clone(), terminal_driver, ctx)
        });

        let (done_tx, done_rx) = futures::channel::oneshot::channel::<()>();
        driver_handle.update(&mut app, |_, ctx| {
            let spawner = ctx.spawner();
            ctx.spawn(
                async move {
                    AgentDriver::load_skills_dirs(&spawner).await;
                    let _ = done_tx.send(());
                },
                |_, _, _| {},
            );
        });
        done_rx.await.expect("loading task should complete");

        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("WARP_SKILL_DIRS") };

        let skill_names = SkillManager::handle(&app).read(&app, |manager: &SkillManager, ctx| {
            manager
                .get_skills_for_working_directory(None, ctx)
                .into_iter()
                .map(|s| s.name.clone())
                .collect::<Vec<_>>()
        });

        assert!(
            skill_names.contains(&"env-skill-rel".to_string()),
            "'env-skill-rel' should load via a relative WARP_SKILL_DIRS entry resolved against the driver's working dir; got: {skill_names:?}"
        );
    });
}

// ── QUALITY-1801: buffered child event vs. ambient-run teardown ─────────────

/// Polls `condition` on a short interval until it returns true, or panics with an
/// actionable message after `timeout` elapses. Used to deterministically await
/// scheduled async work (e.g. a `ctx.spawn`ed eligibility check) without guessing a
/// fixed sleep duration that a loaded test runner could exceed.
async fn poll_until(
    app: &App,
    timeout: Duration,
    description: &str,
    mut condition: impl FnMut(&App) -> bool,
) {
    let deadline = instant::Instant::now() + timeout;
    loop {
        if condition(app) {
            return;
        }
        assert!(
            instant::Instant::now() < deadline,
            "timed out after {timeout:?} waiting for: {description}"
        );
        Timer::after(Duration::from_millis(5)).await;
    }
}

/// Creates a conversation on `terminal_id` and attaches an in-flight mock response
/// stream to it (mirroring an in-progress parent turn), registered through
/// `ai_controller`. Returns the conversation and stream so the caller can drive the
/// stream to completion.
fn conversation_with_in_progress_mock_stream(
    app: &mut App,
    terminal_id: warpui::EntityId,
    ai_controller: &warpui::ModelHandle<crate::ai::blocklist::BlocklistAIController>,
) -> (
    crate::ai::agent::conversation::AIConversationId,
    warpui::ModelHandle<ResponseStream>,
) {
    let stream_id = ResponseStreamId::new_for_test();
    let conversation_id = BlocklistAIHistoryModel::handle(app).update(app, |history, ctx| {
        let conversation_id = history.start_new_conversation(terminal_id, false, false, false, ctx);
        let task_id = history
            .conversation(&conversation_id)
            .unwrap()
            .get_root_task_id()
            .clone();
        history
            .update_conversation_for_new_request_input_for_test(
                RequestInput {
                    conversation_id,
                    input_messages: HashMap::from([(task_id, vec![])]),
                    working_directory: None,
                    model_id: LLMId::from("test-model"),
                    coding_model_id: LLMId::from("test-coding-model"),
                    cli_agent_model_id: LLMId::from("test-cli-agent-model"),
                    computer_use_model_id: LLMId::from("test-computer-use-model"),
                    shared_session_response_initiator: None,
                    request_start_ts: Local::now(),
                    supported_tools_override: None,
                },
                stream_id.clone(),
                terminal_id,
                ctx,
            )
            .unwrap();
        conversation_id
    });
    let stream = app.add_model(|_| ResponseStream::new_for_test(stream_id.clone()));
    ai_controller.update(app, |controller, ctx| {
        controller.register_mock_stream_for_test(stream_id, conversation_id, stream.clone(), ctx);
    });
    (conversation_id, stream)
}

/// Drives `stream` through a successful `Init` + `Finished(Done)` completion via the
/// real controller subscription, matching a parent turn finishing normally.
fn complete_mock_stream_successfully(app: &mut App, stream: &warpui::ModelHandle<ResponseStream>) {
    stream.update(app, |stream, ctx| {
        stream.emit_response_event_for_test(
            warp_multi_agent_api::ResponseEvent {
                r#type: Some(response_event::Type::Init(response_event::StreamInit {
                    request_id: "test-request".to_string(),
                    conversation_id: "test-server-conversation".to_string(),
                    run_id: String::new(),
                })),
            },
            ctx,
        );
        stream.emit_response_event_for_test(
            warp_multi_agent_api::ResponseEvent {
                r#type: Some(response_event::Type::Finished(
                    response_event::StreamFinished {
                        reason: Some(response_event::stream_finished::Reason::Done(
                            response_event::stream_finished::Done {},
                        )),
                        conversation_usage_metadata: None,
                        token_usage: vec![],
                        should_refresh_model_config: false,
                        #[allow(deprecated)]
                        request_cost: None,
                        request_charges: None,
                    },
                )),
            },
            ctx,
        );
    });
}

/// Enqueues a single buffered child message for `conversation_id`, as if a child
/// agent's report had just arrived over the orchestration SSE stream.
fn enqueue_buffered_child_message(
    app: &mut App,
    conversation_id: crate::ai::agent::conversation::AIConversationId,
) {
    OrchestrationEventService::handle(app).update(app, |service, ctx| {
        service.enqueue_event_batch(
            conversation_id,
            vec![PendingEvent {
                event_id: "child-message-1".to_string(),
                source_agent_id: "child".to_string(),
                attempt_count: 0,
                detail: PendingEventDetail::Message {
                    message_id: "message-1".to_string(),
                    addresses: vec!["parent".to_string()],
                    subject: "subject".to_string(),
                    message_body: "child finished".to_string(),
                },
            }],
            ctx,
        );
    });
}

/// Builds a driver wired up (via the real `execute_run`) to observe the given
/// terminal's history but that never submits any query itself, so the caller can
/// drive the conversation manually. `idle_on_complete` controls whether the driver
/// commits to an immediate exit on `Success` (`None`) or defers it (`Some`).
fn driver_wired_for_terminal(
    app: &mut App,
    terminal_view: warpui::ViewHandle<crate::terminal::TerminalView>,
    idle_on_complete: Option<Duration>,
) -> warpui::ModelHandle<AgentDriver> {
    let temp = TempDir::new().unwrap();
    let driver_handle = app.add_model(|ctx| {
        let terminal_driver =
            super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
        let mut driver = AgentDriver::new_for_test(temp.path().to_path_buf(), terminal_driver, ctx);
        // No initial query is submitted; the conversation is driven manually.
        driver.skip_initial_turn = true;
        driver.idle_on_complete = idle_on_complete;
        driver
    });
    // Installs the real history-model subscription under test (including
    // `drop_pending_events_for_exiting_conversation`), without submitting a query.
    let _run_exit_rx = driver_handle.update(app, |driver, ctx| {
        driver.execute_run(AgentRunPrompt::Local(String::new()), ctx)
    });
    driver_handle
}

/// QUALITY-1801 regression: a child agent's message, queued in
/// `OrchestrationEventService` while the parent's own turn is still streaming, must
/// not start a new MAA request once the parent's ambient run has committed to an
/// immediate terminal exit (no `--idle-on-complete` window). This drives the real
/// `AgentDriver` history-model subscription installed by `execute_run` (not just the
/// `OrchestrationEventService` helper directly), so a regression that drops or
/// mis-scopes that wiring is caught here.
#[test]
fn ambient_driver_immediate_exit_blocks_buffered_child_event_from_restarting_maa() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let (terminal_id, ai_controller) = terminal_view.update(&mut app, |view, _| {
            (view.id(), view.ai_controller().clone())
        });

        // No idle window: `Success` commits the run to an immediate exit.
        let _driver = driver_wired_for_terminal(&mut app, terminal_view, None);

        let (conversation_id, stream) =
            conversation_with_in_progress_mock_stream(&mut app, terminal_id, &ai_controller);

        // A child agent's message arrives while the parent's own turn is still
        // streaming: it is queued, not injected (the pre-existing active-stream guard).
        enqueue_buffered_child_message(&mut app, conversation_id);
        ai_controller.read(&app, |controller, ctx| {
            assert!(
                controller.has_active_stream_for_conversation(conversation_id, ctx),
                "the parent's own stream must still be active while its event is buffered"
            );
        });

        // The parent's own turn finishes successfully.
        complete_mock_stream_successfully(&mut app, &stream);
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&ConversationStatus::Success)
            );
        });
        // With no idle window, the driver committed to an immediate exit the instant
        // it observed `Success` above, marking the conversation exiting before the
        // controller's post-stream-cleanup re-check below runs.
        OrchestrationEventService::handle(&app).read(&app, |service, _| {
            assert!(
                service.is_conversation_exiting(conversation_id),
                "the driver should have marked the conversation exiting on immediate Success"
            );
        });

        // The stream's natural completion propagates `AfterStreamFinished`, which is
        // where the controller re-checks pending orchestration events. Since
        // `conversation_ready_for_pending_events` already sees the conversation as
        // exiting, this returns synchronously without going through the async
        // dormant-Claude-wake eligibility check below.
        stream.update(&mut app, |stream, ctx| {
            stream.emit_after_stream_finished_for_test(ctx);
        });

        // The buffered child event must not have started a new request: the run
        // already committed to exiting when the driver observed `Success` above.
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&ConversationStatus::Success),
                "conversation must stay terminal, not flip back to InProgress"
            );
        });
        ai_controller.read(&app, |controller, ctx| {
            assert!(
                !controller.has_active_stream_for_conversation(conversation_id, ctx),
                "no follow-up request should have started"
            );
        });
        // The flag that blocks injection (checked above via `is_conversation_exiting`) is set
        // synchronously by `on_commit`, but the full model-side cleanup that drops queued
        // events lives in `execute_run`'s forwarder, which only runs once the async round trip
        // back from `run_exit`'s internal signal completes — so this is polled rather than
        // checked immediately.
        poll_until(
            &app,
            Duration::from_secs(2),
            "the buffered event to be dropped by the forwarder's model-side cleanup",
            |app| {
                OrchestrationEventService::handle(app).read(app, |service, _| {
                    !service.has_pending_events(conversation_id)
                })
            },
        )
        .await;
    });
}

/// Counterpart to the immediate-exit test above: when the driver has an
/// `--idle-on-complete` window (so it does *not* commit to an immediate exit on
/// `Success`), the buffered child event must still be injected as a normal
/// follow-up once the parent's stream completes — this is the `wait_for_events` /
/// follow-up-turn path the fix must not break.
#[test]
fn ambient_driver_with_idle_window_still_injects_buffered_child_event() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let (terminal_id, ai_controller) = terminal_view.update(&mut app, |view, _| {
            (view.id(), view.ai_controller().clone())
        });

        // A generous idle window: `Success` must not commit the run to an immediate
        // exit, since it is deliberately staying alive for a follow-up.
        let _driver =
            driver_wired_for_terminal(&mut app, terminal_view, Some(Duration::from_secs(300)));

        let (conversation_id, stream) =
            conversation_with_in_progress_mock_stream(&mut app, terminal_id, &ai_controller);

        enqueue_buffered_child_message(&mut app, conversation_id);
        complete_mock_stream_successfully(&mut app, &stream);

        OrchestrationEventService::handle(&app).read(&app, |service, _| {
            assert!(
                !service.is_conversation_exiting(conversation_id),
                "an idle window means the driver must not have committed to exiting"
            );
        });

        stream.update(&mut app, |stream, ctx| {
            stream.emit_after_stream_finished_for_test(ctx);
        });
        // The re-check first goes through an async dormant-Claude-wake eligibility
        // check (`maybe_prepare_local_claude_wake`) that resolves `Ok(None)` for a
        // non-child conversation like this one and falls back to direct injection.
        // Poll for that scheduled work to land instead of guessing a fixed sleep
        // duration a loaded test runner could exceed.
        poll_until(
            &app,
            Duration::from_secs(2),
            "the buffered event to be injected as an InProgress follow-up",
            |app| {
                BlocklistAIHistoryModel::handle(app).read(app, |history, _| {
                    matches!(
                        history.conversation(&conversation_id).map(|c| c.status()),
                        Some(ConversationStatus::InProgress)
                    )
                })
            },
        )
        .await;

        // The buffered event should have been injected as a real follow-up: the
        // conversation is back `InProgress` and the queue has been drained.
        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&ConversationStatus::InProgress),
                "the buffered event should have started a follow-up request"
            );
        });
        OrchestrationEventService::handle(&app).read(&app, |service, _| {
            assert!(!service.has_pending_events(conversation_id));
        });
    });
}

// Lets a test substitute the `IdleWait` used by the next `execute_run` call on this thread, so
// a deferred idle window's deadline can be reached deterministically instead of via a real
// `thread::sleep`. `execute_run` runs synchronously on the same thread as the test that calls
// it, so this thread-local is read exactly once, synchronously, at construction time. Exposed
// to `driver.rs` as `pub(super)`, since `execute_run` (defined there) is the sole reader.
thread_local! {
    static TEST_IDLE_WAIT: std::cell::RefCell<Option<Arc<dyn super::IdleWait>>> =
        const { std::cell::RefCell::new(None) };
}

pub(super) fn test_idle_wait_override() -> Option<Arc<dyn super::IdleWait>> {
    TEST_IDLE_WAIT.with(|cell| cell.borrow().clone())
}

pub(super) fn set_test_idle_wait_override(wait: Option<Arc<dyn super::IdleWait>>) {
    TEST_IDLE_WAIT.with(|cell| *cell.borrow_mut() = wait);
}

// A second, independent gate consulted from inside `execute_run`'s `on_commit` closure,
// immediately after it commits exiting state and before returning (i.e. strictly before
// `end_run_now`/`end_run_after` send the completion value). Lets a test pause deterministically
// in the window where the commit has already happened but the completion signal — and
// everything downstream of it, including the async forwarder that runs model-side cleanup — has
// provably not yet been observed by anything.
thread_local! {
    static TEST_POST_COMMIT_GATE: std::cell::RefCell<Option<Arc<dyn super::IdleWait>>> =
        const { std::cell::RefCell::new(None) };
}

// Read synchronously at `execute_run` construction time (main thread) and captured by value
// into the `on_commit` closure, rather than re-read via this thread-local at commit time: a
// deferred window's commit runs on a background `thread::spawn`, which does not see a
// same-named thread-local set by the test on the main thread.
pub(super) fn test_post_commit_gate() -> Option<Arc<dyn super::IdleWait>> {
    TEST_POST_COMMIT_GATE.with(|cell| cell.borrow().clone())
}

pub(super) fn set_test_post_commit_gate(gate: Option<Arc<dyn super::IdleWait>>) {
    TEST_POST_COMMIT_GATE.with(|cell| *cell.borrow_mut() = gate);
}

impl<T: Send + 'static> super::IdleTimeoutSender<T> {
    /// Overrides the wait mechanism `end_run_after` uses, so a test can control exactly when a
    /// deferred deadline is considered reached instead of depending on a real `thread::sleep`.
    pub(super) fn with_wait(mut self, wait: Arc<dyn super::IdleWait>) -> Self {
        self.wait = wait;
        self
    }
}

/// Test-only [`super::IdleWait`] that blocks until the test releases it, so a deferred idle
/// window's deadline can be reached at a moment of the test's choosing instead of depending on
/// a real, wall-clock-dependent `thread::sleep`. Needed both to make the elapsed-window test
/// below deterministic (a fixed short duration can fire, or fail to have fired yet, before an
/// assertion runs on a loaded test runner) and to construct the specific interleaving it
/// exercises: releasing the timer while a child event's async injection eligibility check is
/// already in flight.
struct ManualIdleWait {
    release_rx: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl super::IdleWait for ManualIdleWait {
    fn wait(&self, _duration: Duration) {
        let _ = self.release_rx.lock().unwrap().recv();
    }
}

fn manual_idle_wait() -> (Arc<ManualIdleWait>, std::sync::mpsc::Sender<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    (
        Arc::new(ManualIdleWait {
            release_rx: std::sync::Mutex::new(rx),
        }),
        tx,
    )
}

/// QUALITY-1801 regression, second gap: an `--idle-on-complete` window that elapses on its
/// own (no follow-up query arrives before the deadline) must also mark the conversation
/// exiting — and that commitment must be visible even to a child event whose async injection
/// eligibility check was already in flight when the window elapsed, not only to one that
/// arrives after everything has settled.
///
/// This exercises the exact race a plain async-forwarder-only design cannot close: the
/// deferred timer fires on a background thread and only marks exiting once its completion
/// signal reaches the model thread through further async plumbing (`ctx.spawn`'s
/// background-executor round trip). An eligibility check that is already mid-flight when the
/// timer fires can find the guard not yet set. `IdleTimeoutSender`'s `on_commit` hook closes
/// this by committing the exiting state synchronously, on the timer's own thread, before it
/// ever touches the completion channel — so the check re-validates against an
/// already-committed state by the time it actually runs.
///
/// Uses `ManualIdleWait` (via `set_test_idle_wait_override`) instead of a real
/// `thread::sleep`-based duration, so exactly when the deadline is considered reached is
/// under the test's control rather than tied to wall-clock timing.
#[test]
fn ambient_driver_elapsed_idle_window_blocks_buffered_child_event_from_restarting_maa() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let (terminal_id, ai_controller) = terminal_view.update(&mut app, |view, _| {
            (view.id(), view.ai_controller().clone())
        });

        let (elapse_wait, elapse_release_tx) = manual_idle_wait();
        let (post_commit_wait, post_commit_release_tx) = manual_idle_wait();
        set_test_idle_wait_override(Some(elapse_wait));
        set_test_post_commit_gate(Some(post_commit_wait));
        let _driver =
            driver_wired_for_terminal(&mut app, terminal_view, Some(Duration::from_secs(300)));
        set_test_idle_wait_override(None);
        set_test_post_commit_gate(None);

        let (conversation_id, stream) =
            conversation_with_in_progress_mock_stream(&mut app, terminal_id, &ai_controller);

        complete_mock_stream_successfully(&mut app, &stream);
        stream.update(&mut app, |stream, ctx| {
            stream.emit_after_stream_finished_for_test(ctx);
        });

        // The deferred window's background timer is blocked on the manual wait, so nothing
        // has committed to exiting yet — deterministically, not because the test happened to
        // check quickly enough.
        OrchestrationEventService::handle(&app).read(&app, |service, _| {
            assert!(
                !service.is_conversation_exiting(conversation_id),
                "the timer is blocked on the manual wait, so nothing has committed yet"
            );
        });

        // A child agent's message arrives while the window is still (deterministically)
        // open: a legitimate follow-up eligibility check starts, which goes through an async
        // dormant-Claude-wake step before it actually injects.
        enqueue_buffered_child_message(&mut app, conversation_id);

        // Release the timer now, while that async eligibility check may still be in flight.
        // The background thread commits exiting and then blocks again on the second
        // (post-commit) gate, strictly *before* sending the completion value — so at this
        // point the commit has provably landed, but the async forwarder that performs
        // model-side cleanup has provably not run (the value it awaits hasn't been sent).
        elapse_release_tx
            .send(())
            .expect("background timer thread should still be waiting on the manual release");

        poll_until(
            &app,
            Duration::from_secs(2),
            "the conversation to be marked exiting once the idle window elapses",
            |app| {
                OrchestrationEventService::handle(app).read(app, |service, _| {
                    service.is_conversation_exiting(conversation_id)
                })
            },
        )
        .await;

        // Deterministically inside the interleaving window now: exiting is committed, and
        // the forwarder cannot have run yet. Give the in-flight eligibility check
        // opportunity to resolve and (incorrectly) inject, continuously asserting the guard
        // holds throughout rather than only at the end.
        let settle_deadline = instant::Instant::now() + Duration::from_millis(300);
        while instant::Instant::now() < settle_deadline {
            BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
                assert_eq!(
                    history.conversation(&conversation_id).map(|c| c.status()),
                    Some(&ConversationStatus::Success),
                    "conversation must stay terminal, not flip back to InProgress"
                );
            });
            Timer::after(Duration::from_millis(5)).await;
        }

        // Release the post-commit gate so the run can finish tearing down normally.
        post_commit_release_tx
            .send(())
            .expect("background timer thread should still be waiting at the post-commit gate");

        ai_controller.read(&app, |controller, ctx| {
            assert!(
                !controller.has_active_stream_for_conversation(conversation_id, ctx),
                "no follow-up request should have started"
            );
        });
    });
}

/// QUALITY-1801 regression, direct proof of the interleaving window: once
/// [`OrchestrationEventService::exit_commit_handle`]'s `commit` has run — exactly what
/// `IdleTimeoutSender`'s `on_commit` hook does, synchronously, on the timer's own thread,
/// before it ever touches the completion channel — the guard must block injection
/// immediately, even before any model-side cleanup
/// (`drop_pending_events_for_exiting_conversation`) has had a chance to run. This isolates,
/// deterministically and without any timer at all, the property the async-forwarder-only
/// design could not guarantee: a check that runs in the gap between the timer deciding to
/// fire and the model callback actually running must still see the commitment.
#[test]
fn exit_commit_handle_blocks_injection_before_model_side_cleanup_runs() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let (terminal_id, ai_controller) = terminal_view.update(&mut app, |view, _| {
            (view.id(), view.ai_controller().clone())
        });

        let (conversation_id, stream) =
            conversation_with_in_progress_mock_stream(&mut app, terminal_id, &ai_controller);
        complete_mock_stream_successfully(&mut app, &stream);
        stream.update(&mut app, |stream, ctx| {
            stream.emit_after_stream_finished_for_test(ctx);
        });

        // Commit directly via the handle, exactly as `on_commit` does on a background timer
        // thread — with no accompanying model-side cleanup call, simulating the moment right
        // after the timer fires but before the async forwarder callback has run.
        let commit_handle = OrchestrationEventService::handle(&app)
            .read(&app, |service, _| service.exit_commit_handle());
        commit_handle.commit(conversation_id);

        // The guard must already block, even though nothing has cleaned up pending events
        // for this conversation (`drop_pending_events_for_exiting_conversation` never ran).
        enqueue_buffered_child_message(&mut app, conversation_id);

        BlocklistAIHistoryModel::handle(&app).read(&app, |history, _| {
            assert_eq!(
                history.conversation(&conversation_id).map(|c| c.status()),
                Some(&ConversationStatus::Success),
                "a directly-committed exit must block injection even without model-side cleanup"
            );
        });
        ai_controller.read(&app, |controller, ctx| {
            assert!(
                !controller.has_active_stream_for_conversation(conversation_id, ctx),
                "no follow-up request should have started"
            );
        });
    });
}

/// Like `driver_wired_for_terminal`, but simulates a *resumed* conversation: the caller
/// already knows `resumed_conversation_id` before `execute_run` is called, mirroring
/// `AgentDriver::new` setting `run_conversation_id` up front for a restored conversation.
/// Such a run never observes `ConversationServerTokenAssigned` (it already has a server
/// token), so it takes a different path than a fresh run to learning its own conversation
/// id.
fn driver_wired_for_resumed_conversation(
    app: &mut App,
    terminal_view: warpui::ViewHandle<crate::terminal::TerminalView>,
    idle_on_complete: Option<Duration>,
    resumed_conversation_id: crate::ai::agent::conversation::AIConversationId,
) -> warpui::ModelHandle<AgentDriver> {
    let temp = TempDir::new().unwrap();
    let driver_handle = app.add_model(|ctx| {
        let terminal_driver =
            super::terminal::TerminalDriver::create_from_existing_view(terminal_view, ctx);
        let mut driver = AgentDriver::new_for_test(temp.path().to_path_buf(), terminal_driver, ctx);
        driver.skip_initial_turn = true;
        driver.idle_on_complete = idle_on_complete;
        driver.run_conversation_id = Some(resumed_conversation_id);
        driver
    });
    let _run_exit_rx = driver_handle.update(app, |driver, ctx| {
        driver.execute_run(AgentRunPrompt::Local(String::new()), ctx)
    });
    driver_handle
}

/// QUALITY-1801 regression: a *resumed* conversation's deferred `--idle-on-complete`
/// window must also commit exiting once it elapses. `execute_run` seeds the thread-safe
/// commit's tracked conversation id from `self.run_conversation_id` precisely because a
/// resumed run already has it at construction time and never takes the
/// `ConversationServerTokenAssigned` branch that a fresh run relies on to learn it.
#[test]
fn ambient_driver_resumed_conversation_elapsed_idle_window_commits_exiting() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal_view = add_window_with_terminal(&mut app, None);
        let (terminal_id, ai_controller) = terminal_view.update(&mut app, |view, _| {
            (view.id(), view.ai_controller().clone())
        });

        // The conversation exists (and its id is known) *before* the driver is
        // constructed, mirroring a resume: the id comes from restoration, not from a
        // `ConversationServerTokenAssigned` event observed during this run.
        let (conversation_id, stream) =
            conversation_with_in_progress_mock_stream(&mut app, terminal_id, &ai_controller);

        // Two gates, as in the interleaving test above: `elapse_wait` controls when the
        // deferred deadline is reached, and `post_commit_wait` pauses `on_commit` strictly
        // *before* the completion value is sent. `on_commit` is the only thing that can set
        // the exiting flag — the forwarder only drops pending events — so a broken
        // (never-populated) thread-safe seed would leave the flag unset forever; these gates
        // just keep the check deterministic rather than racing the background timer.
        let (elapse_wait, elapse_release_tx) = manual_idle_wait();
        let (post_commit_wait, post_commit_release_tx) = manual_idle_wait();
        set_test_idle_wait_override(Some(elapse_wait));
        set_test_post_commit_gate(Some(post_commit_wait));
        let _driver = driver_wired_for_resumed_conversation(
            &mut app,
            terminal_view,
            Some(Duration::from_secs(300)),
            conversation_id,
        );
        set_test_idle_wait_override(None);
        set_test_post_commit_gate(None);

        complete_mock_stream_successfully(&mut app, &stream);
        stream.update(&mut app, |stream, ctx| {
            stream.emit_after_stream_finished_for_test(ctx);
        });

        OrchestrationEventService::handle(&app).read(&app, |service, _| {
            assert!(
                !service.is_conversation_exiting(conversation_id),
                "the timer is blocked on the manual wait, so nothing has committed yet"
            );
        });

        elapse_release_tx
            .send(())
            .expect("background timer thread should still be waiting on the manual release");

        // The background thread runs `on_commit` (which must populate the thread-safe
        // commit using the seed from `self.run_conversation_id`) and then blocks on the
        // post-commit gate, strictly before sending the completion value. The async
        // forwarder therefore cannot have run yet, so this check is a direct, uncontaminated
        // proof of `on_commit`'s own write, not of the forwarder's fallback.
        poll_until(
            &app,
            Duration::from_secs(2),
            "the resumed conversation to be marked exiting once its idle window elapses, \
             via on_commit's seeded conversation id (not the async forwarder)",
            |app| {
                OrchestrationEventService::handle(app).read(app, |service, _| {
                    service.is_conversation_exiting(conversation_id)
                })
            },
        )
        .await;

        // Release the post-commit gate so the run can finish tearing down normally.
        post_commit_release_tx
            .send(())
            .expect("background timer thread should still be waiting at the post-commit gate");
    });
}

#[test]
#[serial_test::serial]
fn openai_api_key_exports_only_api_key_not_base_url() {
    // The OpenAI typed secret should only export OPENAI_API_KEY as an env var.
    // base_url is piped through the structured secret to the harness instead.
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OPENAI_API_KEY") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var("OPENAI_BASE_URL") };
    let secrets = HashMap::from([(
        "openai-key".to_string(),
        ManagedSecretValue::openai_api_key(
            "sk-test-key",
            Some("https://us.api.openai.com/v1".to_string()),
        ),
    )]);
    let env_vars = build_secret_env_vars(&secrets);
    assert_eq!(
        env_vars.get(&OsString::from("OPENAI_API_KEY")),
        Some(&OsString::from("sk-test-key")),
        "OPENAI_API_KEY should be exported from the typed secret"
    );
    assert!(
        !env_vars.contains_key(&OsString::from("OPENAI_BASE_URL")),
        "OPENAI_BASE_URL should NOT be exported as an env var"
    );
}
