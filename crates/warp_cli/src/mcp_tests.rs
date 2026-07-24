use std::ffi::OsStr;

use clap::builder::TypedValueParser;

use super::*;

fn parse_mcp_spec(value: &str) -> Result<MCPSpec, clap::Error> {
    let cmd = clap::Command::new("test");
    let parser = MCPSpecParser;
    parser.parse_ref(&cmd, None, OsStr::new(value))
}

#[test]
fn test_parse_uuid() {
    let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
    let result = parse_mcp_spec(uuid_str).unwrap();
    match result {
        MCPSpec::Uuid(uuid) => assert_eq!(uuid.to_string(), uuid_str),
        other => panic!("Expected Uuid variant, got {other:?}"),
    }
}

#[test]
fn test_parse_well_known_integration_id() {
    let result = parse_mcp_spec("linear").unwrap();
    match result {
        MCPSpec::WellKnown(id) => assert_eq!(id, "linear"),
        other => panic!("Expected WellKnown variant, got {other:?}"),
    }
}

#[test]
fn test_parse_inline_json_cli_server() {
    let json = r#"{"server-name": {"command": "npx", "args": ["-y", "mcp-server"]}}"#;
    let result = parse_mcp_spec(json).unwrap();
    match result {
        MCPSpec::Json(s) => assert_eq!(s, json),
        other => panic!("Expected Json variant, got {other:?}"),
    }
}

#[test]
fn test_parse_inline_json_single_server() {
    let json = r#"{"command": "npx", "args": ["-y", "mcp-server"]}"#;
    let result = parse_mcp_spec(json).unwrap();
    match result {
        MCPSpec::Json(s) => assert_eq!(s, json),
        other => panic!("Expected Json variant, got {other:?}"),
    }
}

#[test]
fn test_parse_inline_json_sse_server() {
    let json = r#"{"url": "http://localhost:3000/mcp", "headers": {"API_KEY": "value"}}"#;
    let result = parse_mcp_spec(json).unwrap();
    match result {
        MCPSpec::Json(s) => assert_eq!(s, json),
        other => panic!("Expected Json variant, got {other:?}"),
    }
}

#[test]
fn test_parse_inline_json_mcp_servers_wrapper() {
    let json = r#"{"mcpServers": {"server-name": {"command": "npx", "args": []}}}"#;
    let result = parse_mcp_spec(json).unwrap();
    match result {
        MCPSpec::Json(s) => assert_eq!(s, json),
        other => panic!("Expected Json variant, got {other:?}"),
    }
}

#[test]
fn test_uuid_takes_precedence_over_json() {
    // A valid UUID should be parsed as UUID, not as JSON
    let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
    let result = parse_mcp_spec(uuid_str).unwrap();
    assert!(matches!(result, MCPSpec::Uuid(_)));
}

#[test]
fn test_bare_identifier_treated_as_well_known() {
    // A bare non-UUID identifier is a well-known managed MCP id: the server
    // owns the set of recognized ids and unknown ones are skipped at run
    // setup, so new ids work without a client change.
    let result = parse_mcp_spec("not-a-valid-uuid").unwrap();
    assert!(matches!(result, MCPSpec::WellKnown(id) if id == "not-a-valid-uuid"));
}

#[test]
fn test_non_identifier_non_json_treated_as_json() {
    // Anything with characters outside [A-Za-z0-9_-] that isn't a file falls
    // through to the inline-JSON path (and fails JSON parsing later).
    let result = parse_mcp_spec("missing-config.json").unwrap();
    assert!(matches!(result, MCPSpec::Json(_)));
}
