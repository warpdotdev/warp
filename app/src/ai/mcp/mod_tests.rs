#[cfg(not(target_family = "wasm"))]
use crate::ai::mcp::parsing::normalize_codex_toml_to_json;
use crate::ai::mcp::parsing::resolve_json;
use crate::ai::mcp::{
    mcp_provider_from_file_path, CLIServer, JsonTemplate, MCPProvider, MCPServer,
    ParsedTemplatableMCPServerResult, ServerSentEvents, StaticEnvVar, StaticHeader,
    TemplatableMCPServer, TemplatableMCPServerInstallation, TemplateVariable, TransportType,
    VariableType, VariableValue,
};
use serde_json;
use std::collections::HashMap;
use warp_managed_secrets::ManagedSecretValue;

#[test]
fn mcp_provider_from_file_path_recognizes_warp_home_path() {
    if let Some(warp_home_mcp_config_file_path) = warp_core::paths::warp_home_mcp_config_file_path()
    {
        assert_eq!(
            mcp_provider_from_file_path(&warp_home_mcp_config_file_path),
            Some(MCPProvider::Warp)
        );
    }
}

#[test]
fn test_mcp_server_config_serialization_excludes_secret_env_values() {
    // Create a CLI server with environment variables containing secrets
    let cli_server = CLIServer {
        command: "npx".to_string(),
        args: vec!["@modelcontextprotocol/server-postgres".to_string()],
        cwd_parameter: Some("/tmp".to_string()),
        static_env_vars: vec![
            StaticEnvVar {
                name: "API_KEY".to_string(),
                value: "SOME_LEAKED_SECRET".to_string(),
            },
            StaticEnvVar {
                name: "DATABASE_URL".to_string(),
                value: "postgresql://user:password@localhost/db".to_string(),
            },
            StaticEnvVar {
                name: "PUBLIC_CONFIG".to_string(),
                value: "not-secret-value".to_string(),
            },
        ],
    };

    let mcp_server = MCPServer {
        transport_type: TransportType::CLIServer(cli_server),
        name: "test-server".to_string(),
        uuid: uuid::Uuid::new_v4(),
    };

    // Test direct serde serialization
    let serialized = serde_json::to_string(&mcp_server).expect("Failed to serialize MCP server");

    // The serialized config should NOT contain the secret values
    assert!(
        !serialized.contains("SOME_LEAKED_SECRET"),
        "Serialized config contains leaked secret value: {serialized}",
    );
    assert!(
        !serialized.contains("password"),
        "Serialized config contains password: {serialized}",
    );
    assert!(
        !serialized.contains("not-secret-value"),
        "Serialized config contains env var value: {serialized}",
    );

    // But should contain the environment variable names/keys
    assert!(
        serialized.contains("API_KEY"),
        "Serialized config should contain env var key 'API_KEY': {serialized}",
    );
    assert!(
        serialized.contains("DATABASE_URL"),
        "Serialized config should contain env var key 'DATABASE_URL': {serialized}",
    );
    assert!(
        serialized.contains("PUBLIC_CONFIG"),
        "Serialized config should contain env var key 'PUBLIC_CONFIG': {serialized}",
    );
}

/// Helper function to create a test TemplatableMCPServerInstallation with custom values
fn create_test_installation(
    name: &str,
    template_json: &str,
    variables: Vec<(&str, &str)>,
) -> TemplatableMCPServerInstallation {
    let template_variables = variables
        .iter()
        .map(|(key, _)| TemplateVariable {
            key: key.to_string(),
            allowed_values: None,
        })
        .collect();

    let variable_values = variables
        .into_iter()
        .map(|(key, value)| {
            (
                key.to_string(),
                VariableValue {
                    variable_type: VariableType::Text,
                    value: value.to_string(),
                },
            )
        })
        .collect();

    let templatable_mcp_server = TemplatableMCPServer {
        uuid: uuid::Uuid::new_v4(),
        name: name.to_string(),
        description: None,
        template: JsonTemplate {
            json: template_json.to_string(),
            variables: template_variables,
        },
        version: 1234567890,
        gallery_data: None,
    };

    TemplatableMCPServerInstallation::new(
        uuid::Uuid::new_v4(),
        templatable_mcp_server,
        variable_values,
    )
}

#[test]
fn test_static_env_var_direct_serialization() {
    // Test direct serialization of StaticEnvVar to ensure skip_serializing works
    let env_var = StaticEnvVar {
        name: "TEST_SECRET".to_string(),
        value: "SOME_LEAKED_SECRET".to_string(),
    };

    let serialized = serde_json::to_string(&env_var).expect("Failed to serialize env var");

    // Should contain the name but not the value due to skip_serializing
    assert!(
        serialized.contains("TEST_SECRET"),
        "Serialized env var should contain name: {serialized}",
    );
    assert!(
        !serialized.contains("SOME_LEAKED_SECRET"),
        "Serialized env var should not contain value due to skip_serializing: {serialized}",
    );
}

#[test]
fn test_static_env_var_deserialization_with_default() {
    // Test that StaticEnvVar can be deserialized properly with default value
    let json = r#"{"name": "API_KEY"}"#;

    let env_var: StaticEnvVar = serde_json::from_str(json).expect("Failed to deserialize env var");

    assert_eq!(env_var.name, "API_KEY");
    assert_eq!(env_var.value, ""); // Should default to empty string
}

#[test]
fn test_sse_server_serialization() {
    // Test that ServerSentEvents transport type serializes correctly
    let sse_server = ServerSentEvents {
        url: "https://example.com/sse".to_string(),
        headers: Default::default(),
    };

    let mcp_server = MCPServer {
        transport_type: TransportType::ServerSentEvents(sse_server),
        name: "sse-server".to_string(),
        uuid: uuid::Uuid::new_v4(),
    };

    let serialized = serde_json::to_string(&mcp_server).expect("Failed to serialize MCP server");

    // Should contain the URL since it's not a secret field
    assert!(
        serialized.contains("https://example.com/sse"),
        "Serialized SSE server should contain URL: {serialized}",
    );
    assert!(
        serialized.contains("sse-server"),
        "Serialized SSE server should contain name: {serialized}",
    );
}

#[test]
fn test_sse_server_with_headers() {
    // Test that ServerSentEvents transport type with headers serializes correctly
    let static_headers = vec![
        StaticHeader {
            name: "Authorization".to_string(),
            value: "Bearer token123".to_string(),
        },
        StaticHeader {
            name: "X-Custom-Header".to_string(),
            value: "custom-value".to_string(),
        },
    ];

    let sse_server = ServerSentEvents {
        url: "https://example.com/sse".to_string(),
        headers: static_headers,
    };

    let mcp_server = MCPServer {
        transport_type: TransportType::ServerSentEvents(sse_server),
        name: "sse-server-with-headers".to_string(),
        uuid: uuid::Uuid::new_v4(),
    };

    // Test to_user_json includes headers
    let user_json = mcp_server.to_user_json();
    assert!(
        user_json.contains("Bearer token123"),
        "User JSON should contain Authorization header value: {user_json}",
    );
    assert!(
        user_json.contains("X-Custom-Header"),
        "User JSON should contain custom header name: {user_json}",
    );
    assert!(
        user_json.contains("custom-value"),
        "User JSON should contain custom header value: {user_json}",
    );

    // Test from_user_json can parse headers
    let parsed_servers =
        MCPServer::from_user_json(&user_json).expect("Failed to parse MCP server from user JSON");
    assert_eq!(parsed_servers.len(), 1);
    let parsed_server = &parsed_servers[0];

    if let TransportType::ServerSentEvents(parsed_sse) = &parsed_server.transport_type {
        assert_eq!(parsed_sse.url, "https://example.com/sse");
        assert_eq!(parsed_sse.headers.len(), 2);
        assert!(parsed_sse
            .headers
            .iter()
            .any(|h| h.name == "Authorization" && h.value == "Bearer token123"));
        assert!(parsed_sse
            .headers
            .iter()
            .any(|h| h.name == "X-Custom-Header" && h.value == "custom-value"));
    } else {
        panic!("Expected ServerSentEvents transport type");
    }
}

#[test]
fn test_sse_server_headers_default() {
    // Test that headers default to empty map when not specified
    let json = r#"{
        "test-server": {
            "url": "https://example.com/sse"
        }
    }"#;

    let servers = MCPServer::from_user_json(json).expect("Failed to parse MCP servers");
    assert_eq!(servers.len(), 1);

    if let TransportType::ServerSentEvents(sse_server) = &servers[0].transport_type {
        assert_eq!(sse_server.url, "https://example.com/sse");
        assert!(
            sse_server.headers.is_empty(),
            "Headers should default to empty"
        );
    } else {
        panic!("Expected ServerSentEvents transport type");
    }
}

#[test]
fn test_hash_consistency() {
    // Create two identical installations and verify they produce the same hash
    let installation1 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    let installation2 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    assert_eq!(
        installation1.hash().expect("hash should succeed"),
        installation2.hash().expect("hash should succeed"),
        "Identical installations should produce the same hash"
    );
}

#[test]
fn test_hash_different_names() {
    // Verify that different names produce different hashes
    let installation1 = create_test_installation(
        "server-one",
        r#"{"server-one":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    let installation2 = create_test_installation(
        "server-two",
        r#"{"server-two":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    assert_ne!(
        installation1.hash().expect("hash should succeed"),
        installation2.hash().expect("hash should succeed"),
        "Installations with different names should produce different hashes"
    );
}

#[test]
fn test_hash_different_variable_values() {
    // Verify that different variable values produce different hashes
    let installation1 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    let installation2 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "different-secret")],
    );

    assert_ne!(
        installation1.hash().expect("hash should succeed"),
        installation2.hash().expect("hash should succeed"),
        "Installations with different variable values should produce different hashes"
    );
}

#[test]
fn test_hash_different_json_templates() {
    // Verify that different JSON templates produce different hashes
    let installation1 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    let installation2 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"python","args":["server.py"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    assert_ne!(
        installation1.hash().expect("hash should succeed"),
        installation2.hash().expect("hash should succeed"),
        "Installations with different JSON templates should produce different hashes"
    );
}

#[test]
fn test_hash_different_variables() {
    // Verify that different variables produce different hashes
    let installation1 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    let installation2 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"TOKEN":"{{TOKEN}}"}}}"#,
        vec![("TOKEN", "secret123")],
    );

    assert_ne!(
        installation1.hash().expect("hash should succeed"),
        installation2.hash().expect("hash should succeed"),
        "Installations with different variables should produce different hashes"
    );
}

#[test]
fn test_hash_multiple_variables_order_independent() {
    // Verify that variable order doesn't affect hash (BTreeMap ensures consistent ordering)
    let installation1 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","env":{"API_KEY":"{{API_KEY}}","TOKEN":"{{TOKEN}}"}}}"#,
        vec![("API_KEY", "secret123"), ("TOKEN", "token456")],
    );

    let installation2 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","env":{"API_KEY":"{{API_KEY}}","TOKEN":"{{TOKEN}}"}}}"#,
        vec![("TOKEN", "token456"), ("API_KEY", "secret123")],
    );

    assert_eq!(
        installation1.hash().expect("hash should succeed"),
        installation2.hash().expect("hash should succeed"),
        "Hash should be order-independent for multiple variables"
    );
}

#[test]
fn test_hash_ignores_installation_uuid() {
    // Verify that installation UUID doesn't affect hash (only name, template, and variable values)
    let installation1 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    let installation2 = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","args":["server"],"env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "secret123")],
    );

    // Even though these have different UUIDs (created separately), hashes should be the same
    assert_ne!(
        installation1.uuid(),
        installation2.uuid(),
        "UUIDs should be different"
    );
    assert_eq!(
        installation1.hash().expect("hash should succeed"),
        installation2.hash().expect("hash should succeed"),
        "Hash should not depend on installation UUID"
    );
}

#[test]
fn test_to_parsed_templatable_mcp_server_result() {
    let mcp_server = MCPServer {
        transport_type: TransportType::CLIServer(CLIServer {
            command: "npx".to_string(),
            args: vec!["@modelcontextprotocol/server-postgres".to_string()],
            cwd_parameter: None,
            static_env_vars: vec![StaticEnvVar {
                name: "API_KEY".to_string(),
                value: "SOME_SECRET".to_string(),
            }],
        }),
        name: "test-server".to_string(),
        uuid: uuid::Uuid::new_v4(),
    };

    let parsed_result = mcp_server.to_parsed_templatable_mcp_server_result();
    let actual_json_value = serde_json::from_str::<serde_json::Value>(
        parsed_result.templatable_mcp_server.template.json.as_str(),
    )
    .unwrap();
    let expected_json_value = serde_json::from_str::<serde_json::Value>(r#"{"test-server":{"command":"npx","args":["@modelcontextprotocol/server-postgres"],"env":{"API_KEY":"{{API_KEY}}"},"working_directory":null}}"#).unwrap();

    assert_eq!(parsed_result.templatable_mcp_server.name, "test-server");
    assert_eq!(actual_json_value, expected_json_value);
    assert_eq!(
        parsed_result
            .templatable_mcp_server
            .template
            .variables
            .len(),
        1
    );
    assert_eq!(
        parsed_result.templatable_mcp_server.template.variables[0].key,
        "API_KEY"
    );

    let variable_values = parsed_result
        .templatable_mcp_server_installation
        .as_ref()
        .unwrap()
        .variable_values();
    assert_eq!(variable_values.len(), 1);
    assert_eq!(variable_values["API_KEY"].variable_type, VariableType::Text);
    assert_eq!(variable_values["API_KEY"].value, "SOME_SECRET");
}

#[test]
fn test_to_parsed_templatable_mcp_server_result_sse_headers() {
    let mcp_server = MCPServer {
        transport_type: TransportType::ServerSentEvents(ServerSentEvents {
            url: "https://example.com/sse".to_string(),
            headers: vec![
                StaticHeader {
                    name: "Authorization".to_string(),
                    value: "Bearer token123".to_string(),
                },
                StaticHeader {
                    name: "X-Custom-Header".to_string(),
                    value: "custom-value".to_string(),
                },
            ],
        }),
        name: "sse-server".to_string(),
        uuid: uuid::Uuid::new_v4(),
    };

    let parsed_result = mcp_server.to_parsed_templatable_mcp_server_result();
    let actual_json_value = serde_json::from_str::<serde_json::Value>(
        parsed_result.templatable_mcp_server.template.json.as_str(),
    )
    .unwrap();
    let expected_json_value = serde_json::from_str::<serde_json::Value>(
        r#"{"sse-server":{"url":"https://example.com/sse","headers":{"Authorization":"{{Authorization}}","X-Custom-Header":"{{X-Custom-Header}}"}}}"#,
    )
    .unwrap();

    assert_eq!(actual_json_value, expected_json_value);

    let mut variable_keys = parsed_result
        .templatable_mcp_server
        .template
        .variables
        .iter()
        .map(|v| v.key.as_str())
        .collect::<Vec<_>>();
    variable_keys.sort();
    assert_eq!(variable_keys, vec!["Authorization", "X-Custom-Header"]);

    let variable_values = parsed_result
        .templatable_mcp_server_installation
        .as_ref()
        .unwrap()
        .variable_values();
    assert_eq!(variable_values["Authorization"].value, "Bearer token123");
    assert_eq!(
        variable_values["Authorization"].variable_type,
        VariableType::Text
    );
    assert_eq!(variable_values["X-Custom-Header"].value, "custom-value");
    assert_eq!(
        variable_values["X-Custom-Header"].variable_type,
        VariableType::Text
    );
}

#[test]
fn test_parse_cli_server_without_args() {
    // MCP configs should work without an explicit "args" field.
    // The args field should default to an empty array.
    let json = r#"{
        "my-server": {
            "command": "uvx",
            "env": {
                "API_KEY": "secret123"
            }
        }
    }"#;

    let servers = MCPServer::from_user_json(json).expect("Failed to parse MCP servers");
    assert_eq!(servers.len(), 1);

    if let TransportType::CLIServer(cli_server) = &servers[0].transport_type {
        assert_eq!(cli_server.command, "uvx");
        assert!(
            cli_server.args.is_empty(),
            "Args should default to empty when not specified"
        );
    } else {
        panic!("Expected CLIServer transport type");
    }
}

#[test]
fn test_parse_cli_server_preserves_explicit_working_directory() {
    // An explicitly-set `working_directory` in a `.mcp.json`-style config must
    // round-trip into `CLIServer.cwd_parameter` so the file-based spawner does
    // not overwrite it with the discovery-root default.
    let json = r#"{
        "my-server": {
            "command": "node",
            "args": ["./tooling/mcp/server.js"],
            "working_directory": "/explicit/override/path"
        }
    }"#;

    let servers = MCPServer::from_user_json(json).expect("Failed to parse MCP servers");
    assert_eq!(servers.len(), 1);

    let TransportType::CLIServer(cli_server) = &servers[0].transport_type else {
        panic!("Expected CLIServer transport type");
    };
    assert_eq!(
        cli_server.cwd_parameter.as_deref(),
        Some("/explicit/override/path"),
        "Explicit working_directory must be preserved through parsing"
    );
}

#[test]
fn test_parse_templatable_cli_server_without_args_and_resolve_json() {
    // Templatable MCP configs should work without an explicit "args" field.
    let json = r#"{
        "my-server": {
            "command": "uvx",
            "env": {
                "API_KEY": "secret123"
            }
        }
    }"#;

    let parsed = ParsedTemplatableMCPServerResult::from_user_json(json)
        .expect("Failed to parse templatable MCP server JSON");
    assert_eq!(parsed.len(), 1);

    let installation = parsed[0]
        .templatable_mcp_server_installation
        .as_ref()
        .expect("Installation should be present when all variables are provided");

    // The resolved JSON should parse successfully via MCPServer::from_user_json
    let resolved = resolve_json(installation);
    let servers = MCPServer::from_user_json(&resolved).expect("Failed to parse resolved MCP JSON");
    assert_eq!(servers.len(), 1);

    if let TransportType::CLIServer(cli_server) = &servers[0].transport_type {
        assert_eq!(cli_server.command, "uvx");
        assert!(
            cli_server.args.is_empty(),
            "Args should default to empty when not specified"
        );
    } else {
        panic!("Expected CLIServer transport type");
    }
}

// ── Codex TOML normalizer tests ────────────────────────────────────────

/// Basic STDIO server: `command` + `args` round-trips cleanly.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_basic_stdio_server() {
    let toml = r#"
    [mcp_servers.context7]
    command = "npx"
    args = ["-y", "@upstash/context7-mcp"]
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let parsed = ParsedTemplatableMCPServerResult::from_user_json(&json)
        .expect("from_user_json should succeed");
    assert_eq!(parsed.len(), 1);
    let server = &parsed[0].templatable_mcp_server;
    assert_eq!(server.name, "context7");
    // No env vars → installation should still be present (no missing variables)
    assert!(
        parsed[0].templatable_mcp_server_installation.is_some(),
        "installation should be present when there are no template variables"
    );
}

/// `env_vars` entries are lowered to `${NAME}` placeholders in the env map.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_env_vars_become_placeholders() {
    let toml = r#"
    [mcp_servers.my_stdio]
    command = "npx"
    args = ["-y", "@example/mcp-server"]
    env_vars = ["MY_API_KEY"]
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    let env = &value["mcp_servers"]["my_stdio"]["env"];
    assert_eq!(
        env["MY_API_KEY"].as_str(),
        Some("${MY_API_KEY}"),
        "env_vars entry should become a ${{NAME}} placeholder"
    );
}

/// Explicit `env` values win over `env_vars` placeholders on collision.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_explicit_env_wins_over_env_vars_on_collision() {
    let toml = r#"
    [mcp_servers.my_stdio]
    command = "npx"
    args = ["-y", "@example/mcp-server"]
    env_vars = ["MY_API_KEY"]

    [mcp_servers.my_stdio.env]
    MY_API_KEY = "literal-value"
    LOG_LEVEL = "info"
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    let env = &value["mcp_servers"]["my_stdio"]["env"];
    // Explicit env wins: literal value, not the placeholder
    assert_eq!(
        env["MY_API_KEY"].as_str(),
        Some("literal-value"),
        "explicit env entry should override env_vars placeholder"
    );
    assert_eq!(
        env["LOG_LEVEL"].as_str(),
        Some("info"),
        "non-colliding explicit env entry should be present"
    );
}

/// `cwd` is mapped to `working_directory` in the output JSON.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_cwd_maps_to_working_directory() {
    let toml = r#"
    [mcp_servers.my_stdio]
    command = "npx"
    args = ["-y", "@example/mcp-server"]
    cwd = "/home/user/project"
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    assert_eq!(
        value["mcp_servers"]["my_stdio"]["working_directory"].as_str(),
        Some("/home/user/project"),
        "cwd should be mapped to working_directory"
    );
}

/// A TOML with one STDIO and one HTTP server produces both in the output.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_mixed_stdio_and_http_servers() {
    let toml = r#"
    [mcp_servers.my_stdio]
    command = "npx"
    args = ["-y", "@example/mcp-server"]

    [mcp_servers.my_http]
    url = "https://example.com/mcp"
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    let servers = value["mcp_servers"]
        .as_object()
        .expect("mcp_servers should be an object");
    assert!(
        servers.contains_key("my_stdio"),
        "STDIO server should be present"
    );
    assert!(
        servers.contains_key("my_http"),
        "HTTP server should be present"
    );
}

/// An HTTP server with only a `url` field round-trips correctly.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_http_url_only() {
    let toml = r#"
    [mcp_servers.my_http]
    url = "https://example.com/mcp"
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    let server = &value["mcp_servers"]["my_http"];
    assert_eq!(
        server["url"].as_str(),
        Some("https://example.com/mcp"),
        "url should be present in output"
    );
}

/// `bearer_token_env_var` is lowered to `Authorization: "Bearer ${VAR}"`.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_http_bearer_token_env_var() {
    let toml = r#"
    [mcp_servers.my_http]
    url = "https://example.com/mcp"
    bearer_token_env_var = "MCP_TOKEN"
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    assert_eq!(
        value["mcp_servers"]["my_http"]["headers"]["Authorization"].as_str(),
        Some("Bearer ${MCP_TOKEN}"),
        "bearer_token_env_var should produce Authorization: Bearer ${{VAR}}"
    );
}

/// `env_http_headers` entries become `header: "${VAR}"` placeholders.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_http_env_http_headers() {
    let toml = r#"
    [mcp_servers.my_http]
    url = "https://example.com/mcp"
    env_http_headers = { "X-Api-Key" = "MCP_API_KEY" }
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    assert_eq!(
        value["mcp_servers"]["my_http"]["headers"]["X-Api-Key"].as_str(),
        Some("${MCP_API_KEY}"),
        "env_http_headers entry should become a ${{VAR}} placeholder"
    );
}

/// `http_headers` static values are passed through verbatim.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_http_static_headers() {
    let toml = r#"
    [mcp_servers.my_http]
    url = "https://example.com/mcp"
    http_headers = { "X-Client" = "codex" }
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    assert_eq!(
        value["mcp_servers"]["my_http"]["headers"]["X-Client"].as_str(),
        Some("codex"),
        "http_headers static value should pass through verbatim"
    );
}

/// `http_headers` wins over `env_http_headers` on collision.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_http_static_headers_win_over_env_headers_on_collision() {
    let toml = r#"
    [mcp_servers.my_http]
    url = "https://example.com/mcp"
    env_http_headers = { "X-Api-Key" = "MCP_API_KEY" }
    http_headers = { "X-Api-Key" = "static-override" }
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    assert_eq!(
        value["mcp_servers"]["my_http"]["headers"]["X-Api-Key"].as_str(),
        Some("static-override"),
        "http_headers should override env_http_headers on collision"
    );
}

/// Entries with neither `command` nor `url` are skipped.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_unknown_entry_skipped() {
    let toml = r#"
    [mcp_servers.my_stdio]
    command = "npx"

    [mcp_servers.mystery]
    some_unknown_field = "value"
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let value: serde_json::Value =
        serde_json::from_str(&json).expect("normalized output should be valid JSON");
    let servers = value["mcp_servers"]
        .as_object()
        .expect("mcp_servers should be an object");
    assert!(
        servers.contains_key("my_stdio"),
        "STDIO server should be present"
    );
    assert!(
        !servers.contains_key("mystery"),
        "entry with neither command nor url should be skipped"
    );
}

/// Full round-trip: TOML with env + env_vars parses into a working installation
/// whose resolved JSON is consumable by `MCPServer::from_user_json`.
#[cfg(not(target_family = "wasm"))]
#[test]
fn test_codex_toml_round_trip_through_from_user_json() {
    let toml = r#"
    [mcp_servers.my_stdio]
    command = "npx"
    args = ["-y", "@example/mcp-server"]

    [mcp_servers.my_stdio.env]
    LOG_LEVEL = "info"
    "#;
    let json = normalize_codex_toml_to_json(toml).expect("normalization should succeed");
    let parsed = ParsedTemplatableMCPServerResult::from_user_json(&json)
        .expect("from_user_json should succeed");
    assert_eq!(parsed.len(), 1);

    let installation = parsed[0]
        .templatable_mcp_server_installation
        .as_ref()
        .expect("installation should be present when all variables have values");

    // The installation's variable values should contain LOG_LEVEL
    let variable_values = installation.variable_values();
    assert_eq!(
        variable_values["LOG_LEVEL"].value, "info",
        "explicit env value should be stored in installation"
    );

    // Resolved JSON should parse as a valid MCPServer
    let resolved = resolve_json(installation);
    let servers =
        MCPServer::from_user_json(&resolved).expect("resolved JSON should parse as MCPServer");
    assert_eq!(servers.len(), 1);
    if let TransportType::CLIServer(cli) = &servers[0].transport_type {
        assert_eq!(cli.command, "npx");
        assert_eq!(cli.args, vec!["-y", "@example/mcp-server"]);
    } else {
        panic!("Expected CLIServer transport type");
    }
}

#[test]
fn test_parse_templatable_sse_headers_and_resolve_json() {
    let json = r#"{
        "sse-server": {
            "url": "https://example.com/sse",
            "headers": {
                "Authorization": "Bearer token123",
                "X-Custom-Header": "custom-value"
            }
        }
    }"#;

    let parsed = ParsedTemplatableMCPServerResult::from_user_json(json)
        .expect("Failed to parse templatable MCP server JSON");
    assert_eq!(parsed.len(), 1);

    let templatable = &parsed[0].templatable_mcp_server;
    let installation = parsed[0]
        .templatable_mcp_server_installation
        .as_ref()
        .expect("Installation should be present when all variables are provided");

    let template_value =
        serde_json::from_str::<serde_json::Value>(templatable.template.json.as_str()).unwrap();
    let expected_template_value = serde_json::from_str::<serde_json::Value>(
        r#"{"sse-server":{"url":"https://example.com/sse","headers":{"Authorization":"{{Authorization}}","X-Custom-Header":"{{X-Custom-Header}}"}}}"#,
    )
    .unwrap();
    assert_eq!(template_value, expected_template_value);

    let variable_values = installation.variable_values();
    assert_eq!(variable_values["Authorization"].value, "Bearer token123");
    assert_eq!(variable_values["X-Custom-Header"].value, "custom-value");

    let resolved_value =
        serde_json::from_str::<serde_json::Value>(&resolve_json(installation)).unwrap();
    let expected_resolved_value = serde_json::from_str::<serde_json::Value>(
        r#"{"sse-server":{"url":"https://example.com/sse","headers":{"Authorization":"Bearer token123","X-Custom-Header":"custom-value"}}}"#,
    )
    .unwrap();
    assert_eq!(resolved_value, expected_resolved_value);
}

// --- Runtime handlebars secret resolution tests ---

fn make_secrets(pairs: Vec<(&str, &str)>) -> HashMap<String, ManagedSecretValue> {
    pairs
        .into_iter()
        .map(|(k, v)| {
            (
                k.to_string(),
                ManagedSecretValue::RawValue {
                    value: v.to_string(),
                },
            )
        })
        .collect()
}

#[test]
fn test_apply_secrets_resolves_explicit_handlebars_in_env_value() {
    // Parser templatizes "API_KEY": "{{secret_one}}" → template has {{API_KEY}},
    // variable value is API_KEY = "{{secret_one}}". apply_secrets renders
    // the explicit {{...}} ref against the secrets map.
    let mut installation = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "{{secret_one}}")],
    );

    let secrets = make_secrets(vec![("secret_one", "real_api_key_value")]);
    installation.apply_secrets(&secrets);

    assert_eq!(
        installation.variable_values()["API_KEY"].value,
        "real_api_key_value"
    );
}

#[test]
fn test_apply_secrets_resolves_bearer_header_with_handlebars() {
    // "Authorization": "Bearer {{my_token}}" → variable value is
    // Authorization = "Bearer {{my_token}}". apply_secrets renders the
    // embedded ref while keeping the Bearer prefix.
    let mut installation = create_test_installation(
        "sse-server",
        r#"{"sse-server":{"url":"https://example.com","headers":{"Authorization":"{{Authorization}}"}}}"#,
        vec![("Authorization", "Bearer {{my_token}}")],
    );

    let secrets = make_secrets(vec![("my_token", "tok_abc123")]);
    installation.apply_secrets(&secrets);

    assert_eq!(
        installation.variable_values()["Authorization"].value,
        "Bearer tok_abc123"
    );
}

#[test]
fn test_apply_secrets_skips_plain_values() {
    // Plain values like "info" contain no {{...}} and should be left unchanged
    // when no secret matches the key name.
    let mut installation = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","env":{"LOG_LEVEL":"{{LOG_LEVEL}}"}}}"#,
        vec![("LOG_LEVEL", "info")],
    );

    let secrets = make_secrets(vec![("some_secret", "value")]);
    installation.apply_secrets(&secrets);

    assert_eq!(installation.variable_values()["LOG_LEVEL"].value, "info");
}

#[test]
fn test_apply_secrets_explicit_refs_take_priority_over_key_match() {
    // If a value contains {{secret_one}} and a secret named API_KEY also exists,
    // the explicit {{secret_one}} should win.
    let mut installation = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "{{secret_one}}")],
    );

    let secrets = make_secrets(vec![
        ("secret_one", "correct_value"),
        ("API_KEY", "wrong_value_from_key_match"),
    ]);
    installation.apply_secrets(&secrets);

    assert_eq!(
        installation.variable_values()["API_KEY"].value,
        "correct_value"
    );
}

#[test]
fn test_apply_secrets_mixed_explicit_and_implicit() {
    // Mixed case: one variable uses explicit {{...}} ref, another uses
    // implicit key-name matching.
    let mut installation = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","env":{"API_KEY":"{{API_KEY}}","LOG_LEVEL":"{{LOG_LEVEL}}"}}}"#,
        vec![("API_KEY", "{{secret_one}}"), ("LOG_LEVEL", "info")],
    );

    let secrets = make_secrets(vec![
        ("secret_one", "resolved_secret"),
        ("LOG_LEVEL", "debug_from_secret"),
    ]);
    installation.apply_secrets(&secrets);

    // API_KEY resolved via explicit handlebars
    assert_eq!(
        installation.variable_values()["API_KEY"].value,
        "resolved_secret"
    );
    // LOG_LEVEL resolved via implicit key-name matching
    assert_eq!(
        installation.variable_values()["LOG_LEVEL"].value,
        "debug_from_secret"
    );
}

#[test]
fn test_apply_secrets_missing_secret_leaves_placeholder() {
    // If the referenced secret doesn't exist, the {{...}} placeholder
    // should remain in the value.
    let mut installation = create_test_installation(
        "test-server",
        r#"{"test-server":{"command":"npx","env":{"API_KEY":"{{API_KEY}}"}}}"#,
        vec![("API_KEY", "{{nonexistent_secret}}")],
    );

    let secrets = make_secrets(vec![]);
    installation.apply_secrets(&secrets);

    assert_eq!(
        installation.variable_values()["API_KEY"].value,
        "{{nonexistent_secret}}"
    );
}
