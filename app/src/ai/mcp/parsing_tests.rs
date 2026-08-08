use std::env;

use super::{ParsedTemplatableMCPServerResult, substitute_env_vars};

fn cleanup_env_vars(vars: &[&str]) {
    for var in vars {
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { env::remove_var(var) };
    }
}

#[test]
fn config_file_json_ignores_unrelated_settings() {
    // ~/.claude.json contains Claude Code app settings, not MCP servers.
    let claude_code_settings = r#"{
        "numStartups": 37,
        "tipsHistory": { "new-user-warmup": 9 },
        "projects": {},
        "officialMarketplaceAutoInstallAttempted": true,
        "sonnet45MigrationComplete": true
    }"#;

    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(claude_code_settings)
        .expect("valid JSON should not error");
    assert!(
        servers.is_empty(),
        "Claude Code settings should not be parsed as MCP servers"
    );
}

#[test]
fn config_file_json_parses_mcp_servers_key() {
    let json = r#"{
        "mcpServers": {
            "github": {
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"]
            }
        }
    }"#;

    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(json)
        .expect("valid JSON should not error");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].templatable_mcp_server.name, "github");
}

#[test]
fn config_file_json_parses_mcp_dot_servers_key() {
    let json = r#"{
        "mcp": {
            "servers": {
                "my-server": { "command": "uvx", "args": ["mcp-server"] }
            }
        }
    }"#;

    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(json)
        .expect("valid JSON should not error");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].templatable_mcp_server.name, "my-server");
}

#[test]
fn config_file_json_parses_mcp_underscore_servers_key() {
    let json = r#"{
        "mcp_servers": {
            "s": { "url": "https://example.com/mcp" }
        }
    }"#;

    let servers = ParsedTemplatableMCPServerResult::from_config_file_json(json)
        .expect("valid JSON should not error");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].templatable_mcp_server.name, "s");
}

#[test]
fn config_file_json_returns_error_for_invalid_json() {
    let result = ParsedTemplatableMCPServerResult::from_config_file_json("not json");
    assert!(result.is_err());
}

#[test]
fn from_user_json_still_accepts_bare_server_map() {
    // The permissive from_user_json should continue to accept bare maps
    // (for UI paste scenarios).
    let json = r#"{
        "github": {
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-github"]
        }
    }"#;

    let servers =
        ParsedTemplatableMCPServerResult::from_user_json(json).expect("should parse bare map");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].templatable_mcp_server.name, "github");
}

#[test]
fn test_substitute_env_vars_success() {
    let test_vars = ["FOO", "BAZ", "REPEATED"];

    // Setup environment variables
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::set_var("FOO", "bar") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::set_var("BAZ", "qux") };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::set_var("REPEATED", "value") };

    // Test 1: Single variable substitution
    let input = r#"{"key": "${FOO}"}"#;
    let result = substitute_env_vars(input).expect("Single variable substitution should succeed");
    assert_eq!(
        result, r#"{"key": "bar"}"#,
        "Single variable FOO should be replaced with 'bar'"
    );

    // Test 2: Multiple different variables
    let input = r#"{"key": "${FOO}", "other": "${BAZ}"}"#;
    let result = substitute_env_vars(input).expect("Multiple variable substitution should succeed");
    assert_eq!(
        result, r#"{"key": "bar", "other": "qux"}"#,
        "Multiple variables FOO and BAZ should be replaced"
    );

    // Test 3: Multiple occurrences of same variable
    let input = r#"{"a": "${REPEATED}", "b": "${REPEATED}", "c": "prefix_${REPEATED}_suffix"}"#;
    let result = substitute_env_vars(input).expect("Repeated variable substitution should succeed");
    assert_eq!(
        result, r#"{"a": "value", "b": "value", "c": "prefix_value_suffix"}"#,
        "All occurrences of REPEATED should be replaced with 'value', including within context"
    );

    // Cleanup
    cleanup_env_vars(&test_vars);
}

#[test]
fn test_substitute_env_vars_missing_or_empty() {
    // Test 1: Missing variable
    // Ensure MISSING_VAR is not set
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::remove_var("MISSING_VAR") };

    let input = r#"{"key": "${MISSING_VAR}"}"#;
    let result = substitute_env_vars(input);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing or empty environment variable: MISSING_VAR"),
        "Error message should mention MISSING_VAR, got: {err_msg}"
    );

    // Test 2: Empty variable
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { env::set_var("EMPTY_VAR", "") };

    let input = r#"{"key": "${EMPTY_VAR}"}"#;
    let result = substitute_env_vars(input);
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Missing or empty environment variable: EMPTY_VAR"),
        "Error message should mention EMPTY_VAR, got: {err_msg}"
    );

    // Cleanup
    cleanup_env_vars(&["EMPTY_VAR"]);
}
