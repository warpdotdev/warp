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
        MCPSpec::Json(_) => panic!("Expected Uuid variant"),
    }
}

#[test]
fn test_parse_inline_json_cli_server() {
    let json = r#"{"server-name": {"command": "npx", "args": ["-y", "mcp-server"]}}"#;
    let result = parse_mcp_spec(json).unwrap();
    match result {
        MCPSpec::Json(s) => assert_eq!(s, json),
        MCPSpec::Uuid(_) => panic!("Expected Json variant"),
    }
}

#[test]
fn test_parse_inline_json_single_server() {
    let json = r#"{"command": "npx", "args": ["-y", "mcp-server"]}"#;
    let result = parse_mcp_spec(json).unwrap();
    match result {
        MCPSpec::Json(s) => assert_eq!(s, json),
        MCPSpec::Uuid(_) => panic!("Expected Json variant"),
    }
}

#[test]
fn test_parse_inline_json_sse_server() {
    let json = r#"{"url": "http://localhost:3000/mcp", "headers": {"API_KEY": "value"}}"#;
    let result = parse_mcp_spec(json).unwrap();
    match result {
        MCPSpec::Json(s) => assert_eq!(s, json),
        MCPSpec::Uuid(_) => panic!("Expected Json variant"),
    }
}

#[test]
fn test_parse_inline_json_mcp_servers_wrapper() {
    let json = r#"{"mcpServers": {"server-name": {"command": "npx", "args": []}}}"#;
    let result = parse_mcp_spec(json).unwrap();
    match result {
        MCPSpec::Json(s) => assert_eq!(s, json),
        MCPSpec::Uuid(_) => panic!("Expected Json variant"),
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
fn test_invalid_uuid_treated_as_json() {
    // An invalid UUID that looks like it could be one should be treated as JSON
    let invalid_uuid = "not-a-valid-uuid";
    let result = parse_mcp_spec(invalid_uuid).unwrap();
    assert!(matches!(result, MCPSpec::Json(_)));
}
