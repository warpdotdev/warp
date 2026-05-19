use super::{CLIServer, MCPServer, ServerSentEvents, StaticEnvVar, TransportType};

#[test]
fn test_mcp_server_config_serialization_excludes_secret_env_values() {
    // Create a CLI server with environment variables containing secrets.
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
    // Test direct serde serialization.
    let serialized = serde_json::to_string(&mcp_server).expect("Failed to serialize MCP server");
    // The serialized config should not contain the secret values.
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
    // The serialized config should contain the environment variable names and keys.
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

#[test]
fn test_static_env_var_direct_serialization() {
    // Test direct serialization of `StaticEnvVar` to ensure `skip_serializing` works.
    let env_var = StaticEnvVar {
        name: "TEST_SECRET".to_string(),
        value: "SOME_LEAKED_SECRET".to_string(),
    };

    let serialized = serde_json::to_string(&env_var).expect("Failed to serialize env var");

    // The serialized value should contain the name but not the value due to `skip_serializing`.
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
    // Test that `StaticEnvVar` can be deserialized properly with its default value.
    let json = r#"{"name": "API_KEY"}"#;

    let env_var: StaticEnvVar = serde_json::from_str(json).expect("Failed to deserialize env var");

    assert_eq!(env_var.name, "API_KEY");
    // The value should default to an empty string.
    assert_eq!(env_var.value, "");
}

#[test]
fn test_sse_server_serialization() {
    // Test that the `ServerSentEvents` transport type serializes correctly.
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

    // The serialized value should contain the URL since it is not a secret field.
    assert!(
        serialized.contains("https://example.com/sse"),
        "Serialized SSE server should contain URL: {serialized}",
    );
    assert!(
        serialized.contains("sse-server"),
        "Serialized SSE server should contain name: {serialized}",
    );
}
