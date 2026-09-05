use super::{
    CLIServer, MCPServer, ServerSentEvents, StaticEnvVar, TemplatableMCPServer, TransportType,
};

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
fn test_templatable_mcp_server_deserialization_without_uuid_succeeds() {
    // Stored/legacy JSON predating the `uuid` field should still deserialize
    // successfully, rather than failing with a missing field error.
    let json = r#"{
        "name": "test-server",
        "description": null,
        "template": {"json": "{}", "variables": []},
        "gallery_data": null
    }"#;

    serde_json::from_str::<TemplatableMCPServer>(json)
        .expect("Failed to deserialize TemplatableMCPServer without uuid");
}

#[test]
fn test_templatable_mcp_server_missing_uuid_fallback_is_stable_across_reloads() {
    // The same uuid-less record should derive the same fallback uuid every time it's
    // deserialized, so that TemplatableMCPServerManager's orphan-installation matching
    // settles instead of repeating forever.
    let json = r#"{
        "name": "test-server",
        "description": null,
        "template": {"json": "{}", "variables": []},
        "gallery_data": null
    }"#;

    let server_a: TemplatableMCPServer = serde_json::from_str(json)
        .expect("Failed to deserialize TemplatableMCPServer without uuid");
    let server_b: TemplatableMCPServer = serde_json::from_str(json)
        .expect("Failed to deserialize TemplatableMCPServer without uuid");

    assert_eq!(
        server_a.uuid, server_b.uuid,
        "the same uuid-less record should derive the same fallback uuid on every reload"
    );
}

#[test]
fn test_templatable_mcp_server_missing_uuid_fallback_differs_by_content() {
    // Two distinct uuid-less records should derive different fallback uuids, so they
    // don't collide onto the same identifier.
    let json_a = r#"{
        "name": "server-a",
        "description": null,
        "template": {"json": "{}", "variables": []},
        "gallery_data": null
    }"#;
    let json_b = r#"{
        "name": "server-b",
        "description": null,
        "template": {"json": "{}", "variables": []},
        "gallery_data": null
    }"#;

    let server_a: TemplatableMCPServer = serde_json::from_str(json_a)
        .expect("Failed to deserialize TemplatableMCPServer without uuid");
    let server_b: TemplatableMCPServer = serde_json::from_str(json_b)
        .expect("Failed to deserialize TemplatableMCPServer without uuid");

    assert_ne!(
        server_a.uuid, server_b.uuid,
        "distinct uuid-less records should not collide on the same fallback uuid"
    );
}

#[test]
fn test_templatable_mcp_server_deserialization_preserves_provided_uuid() {
    // When `uuid` is present, it must be used as-is rather than being overridden by the
    // content-hash fallback.
    let expected_uuid = uuid::Uuid::new_v4();
    let json = format!(
        r#"{{
        "uuid": "{expected_uuid}",
        "name": "test-server",
        "description": null,
        "template": {{"json": "{{}}", "variables": []}},
        "gallery_data": null
    }}"#
    );

    let server: TemplatableMCPServer =
        serde_json::from_str(&json).expect("Failed to deserialize TemplatableMCPServer");

    assert_eq!(server.uuid, expected_uuid);
}
