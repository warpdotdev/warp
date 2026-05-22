use regex::Regex;
use serial_test::serial;

use crate::{ai::mcp::parsing::ParsedTemplatableMCPServerResult, terminal::model::secrets};

use super::MCPServersEditPageView;

#[test]
#[serial]
fn parsed_mcp_server_with_secret_in_args_is_detected() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("sk-test-secret-11265").expect("regex should compile")],
        std::iter::empty(),
    );

    let parsed_servers = ParsedTemplatableMCPServerResult::from_user_json(
        r#"{
            "demo-new-server-bypass": {
                "command": "/usr/bin/true",
                "args": ["--api-key=sk-test-secret-11265"]
            }
        }"#,
    )
    .expect("valid MCP server JSON should parse");

    assert_eq!(parsed_servers.len(), 1);
    assert!(
        MCPServersEditPageView::parsed_templatable_mcp_server_contains_secrets(&parsed_servers[0])
    );
}

#[test]
#[serial]
fn parsed_mcp_server_with_secret_in_env_is_detected() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("sk-test-secret-11265").expect("regex should compile")],
        std::iter::empty(),
    );

    let parsed_servers = ParsedTemplatableMCPServerResult::from_user_json(
        r#"{
            "demo-env-server": {
                "command": "/usr/bin/true",
                "env": {
                    "API_KEY": "sk-test-secret-11265"
                }
            }
        }"#,
    )
    .expect("valid MCP server JSON should parse");

    assert_eq!(parsed_servers.len(), 1);
    assert!(
        MCPServersEditPageView::parsed_templatable_mcp_server_contains_secrets(&parsed_servers[0])
    );
}

#[test]
#[serial]
fn parsed_mcp_server_with_secret_in_headers_is_detected() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("sk-test-secret-11265").expect("regex should compile")],
        std::iter::empty(),
    );

    let parsed_servers = ParsedTemplatableMCPServerResult::from_user_json(
        r#"{
            "demo-http-server": {
                "url": "https://example.com/mcp",
                "headers": {
                    "Authorization": "Bearer sk-test-secret-11265"
                }
            }
        }"#,
    )
    .expect("valid MCP server JSON should parse");

    assert_eq!(parsed_servers.len(), 1);
    assert!(
        MCPServersEditPageView::parsed_templatable_mcp_server_contains_secrets(&parsed_servers[0])
    );
}

#[test]
#[serial]
fn parsed_mcp_server_without_secrets_is_allowed() {
    secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("sk-test-secret-11265").expect("regex should compile")],
        std::iter::empty(),
    );

    let parsed_servers = ParsedTemplatableMCPServerResult::from_user_json(
        r#"{
            "demo-clean-server": {
                "command": "/usr/bin/true",
                "args": ["--version"]
            }
        }"#,
    )
    .expect("valid MCP server JSON should parse");

    assert_eq!(parsed_servers.len(), 1);
    assert!(
        !MCPServersEditPageView::parsed_templatable_mcp_server_contains_secrets(&parsed_servers[0])
    );
}
