use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cloud_object_models::CodeForge;
use futures::channel::oneshot;
use futures::executor::block_on;
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
use warp_graphql::ai::AgentTaskState;
use warp_graphql::mutations::create_managed_mcp_client_config::{
    CreateManagedMcpClientConfigOutput, ManagedMcpTransportKind,
};
use warp_graphql::response_context::ResponseContext;
use warp_managed_secrets::ManagedSecretValue;
use warp_util::standardized_path::StandardizedPath;
use warpui::{App, SingletonEntity as _};

use super::{
    AgentDriver, AgentDriverError, CLIAgentSessionStatus, DebugWindowController, IdleTimeoutSender,
    LEGACY_OZ_PARENT_LISTENER_MANAGED_EXTERNALLY_ENV, LEGACY_OZ_PARENT_STATE_ROOT_ENV,
    OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV, OZ_MESSAGE_LISTENER_STATE_ROOT_ENV,
    PlatformErrorCode, SDKConversationOutputStatus, build_secret_env_vars, debug_turn_task_state,
    idle_window_for_cli_session_status, idle_window_for_terminal_status,
    setup_failure_status_update, terminal_status_log_outcome,
};
use crate::ai::agent::conversation::{AIConversationId, ConversationStatus};
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionResult, AIAgentActionResultType, AIAgentInput, AIAgentOutput,
    AIAgentOutputMessage, ArtifactCreatedData, CancellationReason, MessageId, RenderableAIError,
    UploadArtifactResult,
};
use crate::ai::agent_sdk::task_env_vars;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::cloud_environments::{GithubRepo, SourceRepo};
use crate::ai::mcp::JSONTransportType;
use crate::ai::mcp::builtin::{FACTORY_MCP_INSTALLATION_UUID, FACTORY_MCP_SERVER_NAME};
use crate::ai::mcp::parsing::normalize_mcp_json;
use crate::ai::skills::SkillManager;
use crate::auth::credentials::Credentials;
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

// ── DebugWindowController tests ───────────────────────────────────────────────────

#[test]
fn debug_window_controller_pin_blocks_refresh_and_finish_rearms() {
    let (tx, mut rx) = oneshot::channel::<()>();
    let controller = DebugWindowController::new(IdleTimeoutSender::new(tx));
    let turn_id = AIConversationId::new();

    // A short window so an unpinned controller would fire almost immediately.
    controller.refresh_idle((), Duration::from_millis(30));
    assert!(
        controller.pin_for_turn(turn_id),
        "first pin must be accepted"
    );

    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(
        rx.try_recv().unwrap(),
        None,
        "a pinned debug turn must not let the idle window expire, however long it runs"
    );

    assert!(
        controller.finish_turn(turn_id, (), Duration::from_millis(30)),
        "finishing a tracked turn must report a real transition"
    );
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(
        rx.try_recv().unwrap(),
        Some(()),
        "finishing the last active turn must re-arm the full idle window"
    );
}

#[test]
fn debug_window_controller_duplicate_pin_is_idempotent() {
    let (tx, _rx) = oneshot::channel::<()>();
    let controller = DebugWindowController::new(IdleTimeoutSender::new(tx));
    let turn_id = AIConversationId::new();

    assert!(
        controller.pin_for_turn(turn_id),
        "first pin is a real transition"
    );
    assert!(
        !controller.pin_for_turn(turn_id),
        "a duplicate pin for the same turn id must be a no-op"
    );
}

#[test]
fn debug_window_controller_overlapping_turns_stay_pinned_until_the_last_turn_finishes() {
    let (tx, _rx) = oneshot::channel::<()>();
    let controller = DebugWindowController::new(IdleTimeoutSender::new(tx));
    let first_turn = AIConversationId::new();
    let second_turn = AIConversationId::new();

    assert!(controller.pin_for_turn(first_turn));
    assert!(controller.pin_for_turn(second_turn));
    assert!(controller.finish_turn(first_turn, (), Duration::from_secs(60)));
    assert!(controller.is_pinned());
    assert!(controller.finish_turn(second_turn, (), Duration::from_secs(60)));
    assert!(!controller.is_pinned());
}

#[test]
fn debug_turn_task_state_maps_timeline_states() {
    let blocked = ConversationStatus::Blocked {
        blocked_action: "approval".to_string(),
    };
    let cases = [
        (
            ConversationStatus::InProgress,
            Some(AgentTaskState::InProgress),
        ),
        (ConversationStatus::Success, Some(AgentTaskState::Succeeded)),
        (ConversationStatus::Error, Some(AgentTaskState::Error)),
        (
            ConversationStatus::Cancelled,
            Some(AgentTaskState::Cancelled),
        ),
        (blocked, Some(AgentTaskState::Blocked)),
        (ConversationStatus::TransientError, None),
        (ConversationStatus::WaitingForEvents, None),
    ];

    for (status, expected) in cases {
        assert_eq!(debug_turn_task_state(&status), expected);
    }
}
#[test]
fn debug_window_controller_duplicate_finish_is_idempotent() {
    let (tx, mut rx) = oneshot::channel::<()>();
    let controller = DebugWindowController::new(IdleTimeoutSender::new(tx));
    let turn_id = AIConversationId::new();

    controller.pin_for_turn(turn_id);
    assert!(controller.finish_turn(turn_id, (), Duration::from_millis(30)));
    assert!(
        !controller.finish_turn(turn_id, (), Duration::from_millis(30)),
        "a duplicate terminal event for the same turn id must be a no-op"
    );

    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(
        rx.try_recv().unwrap(),
        Some(()),
        "the single real finish must still re-arm exactly one full window"
    );
}

#[test]
fn debug_window_controller_finish_of_unknown_turn_is_a_no_op() {
    let (tx, mut rx) = oneshot::channel::<()>();
    let controller = DebugWindowController::new(IdleTimeoutSender::new(tx));

    controller.refresh_idle((), Duration::from_millis(30));
    assert!(
        !controller.finish_turn(AIConversationId::new(), (), Duration::from_millis(30)),
        "finishing a turn id that was never pinned must be a no-op"
    );

    // The unrelated finish_turn call must not have disturbed the already-armed window.
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(rx.try_recv().unwrap(), Some(()));
}

#[test]
fn debug_window_controller_refresh_is_inert_while_pinned() {
    // Both refresh entry points — the viewer-input keystroke path and the general re-arm path —
    // must be no-ops while a debug turn is pinning the window, for the same reason: neither may
    // move a deadline that finish_turn is about to race by re-arming the full interval anyway.
    let (tx, _rx) = oneshot::channel::<()>();
    let controller = DebugWindowController::new(IdleTimeoutSender::new(tx));

    controller.refresh_idle((), Duration::from_secs(60));
    controller.pin_for_turn(AIConversationId::new());
    assert_eq!(
        controller.refresh_from_last_armed(),
        None,
        "a viewer-input refresh must not move the deadline while a debug turn is pinning it"
    );

    let controller = DebugWindowController::new(IdleTimeoutSender::new(oneshot::channel::<()>().0));
    controller.pin_for_turn(AIConversationId::new());
    assert!(
        !controller.refresh_idle((), Duration::from_millis(30)),
        "refreshing while pinned must not arm a deadline that finish_turn would then race"
    );
}

// ── Terminal-status idle window routing ────────────────────────────────────────────

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
    assert_eq!(
        env_vars.get(&OsString::from(LEGACY_OZ_PARENT_STATE_ROOT_ENV)),
        Some(&OsString::from("/tmp/message-listener-root"))
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
