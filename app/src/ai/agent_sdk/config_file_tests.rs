#![cfg(not(target_family = "wasm"))]

use std::io::Write as _;

use serde_json::json;
use warp_cli::mcp::MCPSpec;

use crate::ai::ambient_agents::AgentConfigSnapshot;

fn write_temp(suffix: &str, contents: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::Builder::new().suffix(suffix).tempfile().unwrap();
    file.write_all(contents.as_bytes()).unwrap();
    file
}

#[test]
fn loads_json_and_validates_mcp_servers() {
    let contents = json!({
        "model_id": "gpt-4o",
        "environment_id": "env-123",
        "base_prompt": "be helpful",
        "mcp_servers": {
            "s": { "command": "npx", "args": [] }
        }
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    assert_eq!(loaded.file.model_id.as_deref(), Some("gpt-4o"));
    assert_eq!(loaded.file.environment_id.as_deref(), Some("env-123"));
    assert_eq!(loaded.file.base_prompt.as_deref(), Some("be helpful"));
    assert!(loaded.file.mcp_servers.is_some());
    assert!(loaded.file.mcp_servers.as_ref().unwrap().contains_key("s"));
}

#[test]
fn loads_yaml() {
    let contents = r#"
model_id: gpt-4o
mcp_servers:
  s:
    command: npx
    args: []
"#;

    let file = write_temp(".yaml", contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    assert_eq!(loaded.file.model_id.as_deref(), Some("gpt-4o"));
    assert!(loaded.file.mcp_servers.as_ref().unwrap().contains_key("s"));
}

#[test]
fn unknown_keys_are_rejected() {
    let contents = json!({
        "model_id": "gpt-4o",
        "typo_model": "oops"
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let err = super::load_config_file(file.path()).unwrap_err();
    let err_str = format!("{err:#}");
    assert!(err_str.contains("Supported keys"));
}

#[test]
fn mcp_must_be_under_mcp_servers_key() {
    let contents = json!({
        "model_id": "gpt-4o",
        "mcpServers": { "s": { "command": "npx", "args": [] } }
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let err = super::load_config_file(file.path()).unwrap_err();
    let err_str = format!("{err:#}");
    assert!(err_str.contains("Supported keys"));
}

#[test]
fn merge_precedence_cli_over_file_and_merges_mcp() {
    let contents = json!({
        "model_id": "file-model",
        "mcp_servers": {
            "a": { "url": "https://example.com/mcp" }
        }
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    let cli = AgentConfigSnapshot {
        name: Some("cli-name".to_string()),
        environment_id: None,
        model_id: Some("cli-model".to_string()),
        base_prompt: None,
        mcp_servers: Some(serde_json::Map::from_iter([(
            "a".to_string(),
            json!({"command": "npx", "args": []}),
        )])),
        profile_id: None,
        worker_host: None,
        skill_spec: None,
        computer_use_enabled: None,
        harness: None,
        harness_auth_secrets: None,
    };

    let merged = super::merge_with_precedence(Some(&loaded), cli);

    assert_eq!(merged.name.as_deref(), Some("cli-name"));
    assert_eq!(merged.model_id.as_deref(), Some("cli-model"));

    let a = merged.mcp_servers.as_ref().unwrap().get("a").unwrap();
    assert_eq!(a.get("command").and_then(|v| v.as_str()), Some("npx"));
}

#[test]
fn file_empty_mcp_servers_is_loaded_as_empty_map() {
    let contents = json!({
        "mcp_servers": {}
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    assert!(loaded.file.mcp_servers.is_some());
    assert!(loaded.file.mcp_servers.as_ref().unwrap().is_empty());
}

#[test]
fn mcp_servers_map_converts_to_runtime_specs() {
    let contents = json!({
        "mcp_servers": {
            "existing": { "warp_id": "550e8400-e29b-41d4-a716-446655440000" },
            "ephemeral": { "command": "npx", "args": [] }
        }
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    let map = loaded.file.mcp_servers.as_ref().unwrap();
    let specs = super::mcp_specs_from_mcp_servers(map).unwrap();

    assert!(specs.iter().any(|s| matches!(s, MCPSpec::Uuid(_))));
    assert!(specs.iter().any(|s| matches!(s, MCPSpec::Json(_))));
}

#[test]
fn loads_computer_use_enabled_from_json() {
    let contents = json!({
        "computer_use_enabled": true
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    assert_eq!(loaded.file.computer_use_enabled, Some(true));
}

#[test]
fn loads_computer_use_enabled_from_yaml() {
    let contents = "computer_use_enabled: false\n";

    let file = write_temp(".yaml", contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    assert_eq!(loaded.file.computer_use_enabled, Some(false));
}

#[test]
fn merge_precedence_cli_computer_use_enabled_over_file() {
    let contents = json!({
        "computer_use_enabled": false
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    let cli = AgentConfigSnapshot {
        computer_use_enabled: Some(true),
        ..Default::default()
    };

    let merged = super::merge_with_precedence(Some(&loaded), cli);

    assert_eq!(merged.computer_use_enabled, Some(true));
}

#[test]
fn merge_precedence_file_computer_use_enabled_when_cli_none() {
    let contents = json!({
        "computer_use_enabled": true
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    let cli = AgentConfigSnapshot::default();

    let merged = super::merge_with_precedence(Some(&loaded), cli);

    assert_eq!(merged.computer_use_enabled, Some(true));
}
