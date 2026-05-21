use serde_json::{json, Map, Value};
use warp_cli::mcp::MCPSpec;

use super::build_mcp_servers_from_specs;

fn build(specs: Vec<MCPSpec>) -> Map<String, Value> {
    build_mcp_servers_from_specs(&specs)
        .expect("builder should not error")
        .unwrap_or_default()
}

#[test]
fn empty_specs_returns_none() {
    assert!(build_mcp_servers_from_specs(&[]).unwrap().is_none());
}

#[test]
fn uuid_spec_is_coerced_to_warp_id() {
    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let servers = build(vec![MCPSpec::Uuid(uuid)]);

    let entry = servers.get(&uuid.to_string()).unwrap();
    assert_eq!(
        entry["warp_id"].as_str(),
        Some("550e8400-e29b-41d4-a716-446655440000")
    );
}

#[test]
fn wrapper_mcp_servers_is_unpacked() {
    let spec = json!({
        "mcpServers": {
            "github": { "command": "npx", "args": ["-y", "server"] }
        }
    })
    .to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert!(servers.contains_key("github"));
    assert_eq!(servers["github"]["command"].as_str(), Some("npx"));
    assert!(servers.get("mcpServers").is_none());
}

#[test]
fn wrapper_mcp_servers_snake_case_is_unpacked() {
    let spec = json!({
        "mcp_servers": {
            "s": { "url": "https://example.com/mcp" }
        }
    })
    .to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert!(servers.contains_key("s"));
    assert_eq!(
        servers["s"]["url"].as_str(),
        Some("https://example.com/mcp")
    );
    assert!(servers.get("mcp_servers").is_none());
}

#[test]
fn wrapper_servers_is_unpacked() {
    let spec = json!({
        "servers": {
            "s": { "command": "python", "args": ["mcp.py"] }
        }
    })
    .to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert!(servers.contains_key("s"));
    assert_eq!(servers["s"]["command"].as_str(), Some("python"));
    assert!(servers.get("servers").is_none());
}

#[test]
fn wrapper_mcp_dot_servers_is_unpacked() {
    let spec = json!({
        "mcp": {
            "servers": {
                "s": { "url": "https://example.com/mcp" }
            }
        }
    })
    .to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert!(servers.contains_key("s"));
    assert_eq!(
        servers["s"]["url"].as_str(),
        Some("https://example.com/mcp")
    );
    assert!(servers.get("mcp").is_none());
}

#[test]
fn plain_map_is_accepted() {
    let spec = json!({
        "github": { "command": "npx", "args": [] },
        "remote": { "url": "https://example.com/mcp" }
    })
    .to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert!(servers.contains_key("github"));
    assert!(servers.contains_key("remote"));
}

#[test]
fn missing_outer_braces_is_accepted() {
    // Emulate copying docs that omit the top-level `{}`.
    let full = json!({
        "mcpServers": {
            "s": { "command": "npx", "args": [] }
        }
    })
    .to_string();

    let inner = &full[1..full.len() - 1];
    let spec = format!("  {inner}  ");

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert!(servers.contains_key("s"));
    assert_eq!(servers["s"]["command"].as_str(), Some("npx"));
}

#[test]
fn single_server_shorthand_command_is_wrapped() {
    let spec = json!({ "command": "npx", "args": [] }).to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert_eq!(servers.len(), 1);
    let (_name, config) = servers.iter().next().unwrap();
    assert_eq!(config["command"].as_str(), Some("npx"));
}

#[test]
fn command_without_args_is_accepted() {
    // args should be optional for command-based MCP servers
    let spec = json!({
        "mcpServers": {
            "s": { "command": "uvx" }
        }
    })
    .to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert!(servers.contains_key("s"));
    assert_eq!(servers["s"]["command"].as_str(), Some("uvx"));
}

#[test]
fn single_server_shorthand_url_is_wrapped() {
    let spec = json!({ "url": "https://example.com/mcp" }).to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);

    assert_eq!(servers.len(), 1);
    let (_name, config) = servers.iter().next().unwrap();
    assert_eq!(config["url"].as_str(), Some("https://example.com/mcp"));
}

#[test]
fn merge_multiple_specs_and_duplicate_name_errors() {
    let s1 = json!({ "mcpServers": { "a": { "command": "npx", "args": [] } } }).to_string();
    let s2 = json!({ "mcpServers": { "b": { "url": "https://example.com/mcp" } } }).to_string();

    let servers = build(vec![MCPSpec::Json(s1.clone()), MCPSpec::Json(s2)]);
    assert!(servers.contains_key("a"));
    assert!(servers.contains_key("b"));

    let err =
        build_mcp_servers_from_specs(&[MCPSpec::Json(s1.clone()), MCPSpec::Json(s1)]).unwrap_err();

    assert!(err.to_string().contains("Duplicate MCP server name 'a'"));
}

#[test]
fn preserves_escaped_strings_in_env_values() {
    let spec = json!({
        "mcpServers": {
            "s": {
                "command": "npx",
                "args": [],
                "env": {
                    "TOKEN": "a\"b\\c\n"
                }
            }
        }
    })
    .to_string();

    let servers = build(vec![MCPSpec::Json(spec)]);
    let token = servers["s"]["env"]["TOKEN"].as_str().unwrap();

    // `serde_json` will decode escapes.
    assert_eq!(token, "a\"b\\c\n");
}

#[test]
fn validation_rejects_invalid_entries() {
    // Both command and url.
    let spec = json!({
        "mcpServers": {
            "bad": { "command": "npx", "url": "https://example.com" }
        }
    })
    .to_string();

    let err = build_mcp_servers_from_specs(&[MCPSpec::Json(spec)]).unwrap_err();
    assert!(err
        .to_string()
        .contains("must have exactly one of: 'warp_id', 'command', or 'url'"));

    // warp_id must be a UUID string.
    let spec = json!({ "mcpServers": { "bad": { "warp_id": "not-a-uuid" } } }).to_string();
    let err = build_mcp_servers_from_specs(&[MCPSpec::Json(spec)]).unwrap_err();
    assert!(err.to_string().contains("field 'warp_id' must be a UUID"));

    // args must be array.
    let spec = json!({ "mcpServers": { "bad": { "command": "npx", "args": "nope" } } }).to_string();
    let err = build_mcp_servers_from_specs(&[MCPSpec::Json(spec)]).unwrap_err();
    assert!(err.to_string().contains("field 'args' must be an array"));

    // args entries must be strings.
    let spec = json!({ "mcpServers": { "bad": { "command": "npx", "args": [1] } } }).to_string();
    let err = build_mcp_servers_from_specs(&[MCPSpec::Json(spec)]).unwrap_err();
    assert!(err.to_string().contains("args[0]"));

    // env values must be strings.
    let spec = json!({
        "mcpServers": { "bad": { "command": "npx", "args": [], "env": { "X": 1 } } }
    })
    .to_string();
    let err = build_mcp_servers_from_specs(&[MCPSpec::Json(spec)]).unwrap_err();
    assert!(err.to_string().contains("env.X"));

    // headers values must be strings.
    let spec = json!({
        "mcpServers": { "bad": { "url": "https://example.com", "headers": { "X": 1 } } }
    })
    .to_string();
    let err = build_mcp_servers_from_specs(&[MCPSpec::Json(spec)]).unwrap_err();
    assert!(err.to_string().contains("headers.X"));

    // server config must be an object.
    let spec = json!({ "mcpServers": { "bad": 1 } }).to_string();
    let err = build_mcp_servers_from_specs(&[MCPSpec::Json(spec)]).unwrap_err();
    assert!(err.to_string().contains("config must be a JSON object"));
}

#[test]
fn serializes_mcp_servers_as_object_not_string() {
    use crate::ai::agent::UserQueryMode;
    use crate::ai::ambient_agents::AgentConfigSnapshot;
    use crate::server::server_api::ai::SpawnAgentRequest;

    let uuid = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let mcp_servers = build_mcp_servers_from_specs(&[MCPSpec::Uuid(uuid)])
        .unwrap()
        .unwrap();

    let request = SpawnAgentRequest {
        prompt: "hello".to_string(),
        mode: UserQueryMode::Normal,
        config: Some(AgentConfigSnapshot {
            mcp_servers: Some(mcp_servers),
            ..Default::default()
        }),
        title: None,
        team: None,
        agent_identity_uid: None,
        skill: None,
        attachments: vec![],
        interactive: None,
        parent_run_id: None,
        runtime_skills: vec![],
        referenced_attachments: vec![],
        conversation_id: None,
        initial_snapshot_token: None,
        snapshot_disabled: None,
    };

    let value = serde_json::to_value(&request).unwrap();

    let config = value.get("config").unwrap();
    let mcp_servers = config.get("mcp_servers").unwrap();

    assert!(mcp_servers.is_object());
    assert!(mcp_servers.get("mcpServers").is_none());

    let server = mcp_servers.get(uuid.to_string()).unwrap();
    assert_eq!(
        server.get("warp_id").unwrap(),
        &Value::String(uuid.to_string())
    );
}
