use super::ParsedTemplatableMCPServerResult;

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
