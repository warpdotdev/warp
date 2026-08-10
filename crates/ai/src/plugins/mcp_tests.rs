use serde_json::{Value, json};

use super::*;
use crate::plugins::manifest::AGENT_PLUGINS_VERSION_1_0_0;

const VENDORED_MCP_SCHEMA: &str = include_str!("schema/1.0.0/mcp.schema.json");

fn config(servers: Value) -> String {
    json!({ "$schema": MCP_SCHEMA_1_0_0, "mcpServers": servers }).to_string()
}

fn parse(servers: Value) -> ParsedPluginMcp {
    parse_plugin_mcp(&config(servers), AGENT_PLUGINS_VERSION_1_0_0).expect("top level is valid")
}

/// Parses a single entry and returns the diagnostic that disabled it, if any.
fn entry_diagnostic(entry: Value) -> Option<PluginDiagnostic> {
    let parsed = parse(json!({ "server": entry }));
    parsed.diagnostics.into_iter().next()
}

#[test]
fn vendored_mcp_schema_matches_the_canonical_identifier() {
    let schema: Value = serde_json::from_str(VENDORED_MCP_SCHEMA).unwrap();
    assert_eq!(schema["$id"], MCP_SCHEMA_1_0_0);
    assert_eq!(schema["properties"]["$schema"]["const"], MCP_SCHEMA_1_0_0);
    assert_eq!(schema["additionalProperties"], Value::Bool(false));
}

#[test]
fn permitted_fields_match_the_vendored_schema() {
    let schema: Value = serde_json::from_str(VENDORED_MCP_SCHEMA).unwrap();
    let field_names = |pointer: &str| {
        let mut names: Vec<String> = schema
            .pointer(pointer)
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{pointer} exists"))
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    };
    let sorted = |fields: &[&str]| {
        let mut owned: Vec<String> = fields.iter().map(|field| (*field).to_owned()).collect();
        owned.sort();
        owned
    };

    assert_eq!(
        sorted(PERMITTED_TOP_LEVEL_FIELDS),
        field_names("/properties")
    );
    assert_eq!(
        sorted(PERMITTED_STDIO_FIELDS),
        field_names("/$defs/stdioServer/properties")
    );
    assert_eq!(
        sorted(PERMITTED_HTTP_FIELDS),
        field_names("/$defs/streamableHttpServer/properties")
    );
}

#[test]
fn empty_mcp_servers_object_is_valid() {
    let parsed = parse(json!({}));
    assert!(parsed.servers.is_empty());
    assert!(parsed.diagnostics.is_empty());
}

#[test]
fn stdio_entry_parses_every_field() {
    let parsed = parse(json!({
        "local-validator": {
            "type": "stdio",
            "command": "./bin/validator",
            "args": ["--data", "${PLUGIN_DATA}/validator"],
            "env": { "CONFIG": "${PLUGIN_ROOT}/config.json" },
            "cwd": "${PLUGIN_ROOT}",
        },
    }));
    assert!(parsed.diagnostics.is_empty());

    let server = parsed.servers.first().unwrap();
    assert_eq!(server.name, "local-validator");
    let PluginMcpTransport::Stdio {
        command,
        args,
        env,
        cwd,
    } = &server.transport
    else {
        panic!("expected a stdio transport");
    };
    assert_eq!(command, "./bin/validator");
    assert_eq!(
        args,
        &["--data".to_owned(), "${PLUGIN_DATA}/validator".to_owned()]
    );
    assert_eq!(env["CONFIG"], "${PLUGIN_ROOT}/config.json");
    assert_eq!(cwd.as_deref(), Some("${PLUGIN_ROOT}"));
}

#[test]
fn streamable_http_entry_preserves_literal_url_and_headers() {
    let parsed = parse(json!({
        "deployment-api": {
            "type": "streamable-http",
            "url": "https://deploy.example.com/mcp",
            "headers": { "X-Tenant": "public-tenant" },
        },
    }));
    assert!(parsed.diagnostics.is_empty());

    let PluginMcpTransport::StreamableHttp { url, headers } = &parsed.servers[0].transport else {
        panic!("expected a streamable-http transport");
    };
    assert_eq!(url, "https://deploy.example.com/mcp");
    assert_eq!(
        headers,
        &[("X-Tenant".to_owned(), "public-tenant".to_owned())]
    );
}

/// §7.2.2(2): a bad top level disables MCP for the plugin and leaves other components alone.
#[test]
fn top_level_violations_disable_mcp_for_the_plugin() {
    let cases: Vec<(&str, String, PluginDiagnosticCode)> = vec![
        (
            "not JSON",
            "{".to_owned(),
            PluginDiagnosticCode::McpInvalidJson,
        ),
        (
            "not an object",
            "[]".to_owned(),
            PluginDiagnosticCode::McpInvalidTopLevel,
        ),
        (
            "missing $schema",
            json!({ "mcpServers": {} }).to_string(),
            PluginDiagnosticCode::McpUnsupportedSchema,
        ),
        (
            "unrelated $schema",
            json!({ "$schema": "https://example.com/mcp.json", "mcpServers": {} }).to_string(),
            PluginDiagnosticCode::McpUnsupportedSchema,
        ),
        (
            "missing mcpServers",
            json!({ "$schema": MCP_SCHEMA_1_0_0 }).to_string(),
            PluginDiagnosticCode::McpInvalidTopLevel,
        ),
        (
            "mcpServers wrong type",
            json!({ "$schema": MCP_SCHEMA_1_0_0, "mcpServers": [] }).to_string(),
            PluginDiagnosticCode::McpInvalidTopLevel,
        ),
        (
            "extra top-level field",
            json!({ "$schema": MCP_SCHEMA_1_0_0, "mcpServers": {}, "extra": 1 }).to_string(),
            PluginDiagnosticCode::McpInvalidTopLevel,
        ),
    ];

    for (case, content, expected) in cases {
        let diagnostic = parse_plugin_mcp(&content, AGENT_PLUGINS_VERSION_1_0_0)
            .err()
            .unwrap_or_else(|| panic!("{case}: expected MCP to be disabled"));
        assert_eq!(diagnostic.code, expected, "{case}");
    }
}

/// §10.1: `mcp.json` must target the same Agent Plugins version as `plugin.json`.
#[test]
fn version_mismatch_with_the_manifest_disables_mcp() {
    let content = json!({
        "$schema": "https://agent-plugins.org/schemas/1.1.0/mcp.schema.json",
        "mcpServers": {},
    })
    .to_string();
    let diagnostic = parse_plugin_mcp(&content, AGENT_PLUGINS_VERSION_1_0_0).unwrap_err();
    assert_eq!(diagnostic.code, PluginDiagnosticCode::McpVersionMismatch);
}

/// A Factory-shaped file in a plugin root disables plugin MCP with its own diagnostic, so the
/// author is told that managed servers cannot arrive through a plugin package.
#[test]
fn factory_schema_in_a_plugin_root_disables_plugin_mcp() {
    let content = json!({
        "$schema": crate::plugins::factory_mcp::FACTORY_MCP_SCHEMA_1_0_0,
        "mcpServers": {
            "search": { "type": "managed", "warpId": "00000000-0000-0000-0000-000000000000" },
        },
    })
    .to_string();
    let diagnostic = parse_plugin_mcp(&content, AGENT_PLUGINS_VERSION_1_0_0).unwrap_err();
    assert_eq!(
        diagnostic.code,
        PluginDiagnosticCode::McpFactorySchemaInPluginRoot
    );
    assert!(diagnostic.reason.contains("managed"));
}

/// §7.2.2(3): an invalid entry is skipped and every other entry still loads.
#[test]
fn invalid_entries_are_isolated_from_valid_ones() {
    let parsed = parse(json!({
        "good": { "type": "stdio", "command": "server" },
        "bad": { "type": "stdio" },
        "also-good": { "type": "streamable-http", "url": "https://example.com/mcp" },
    }));

    let names: Vec<&str> = parsed
        .servers
        .iter()
        .map(|server| server.name.as_str())
        .collect();
    assert_eq!(names, vec!["also-good", "good"]);
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(
        parsed.diagnostics[0].component.as_deref(),
        Some("bad"),
        "the diagnostic must name the entry it disabled"
    );
}

/// §7.2.2(4): an unsupported transport is skipped with its own code, not treated as malformed.
#[test]
fn legacy_sse_is_skipped_as_an_unsupported_transport() {
    let parsed = parse(json!({
        "legacy-events": { "type": "sse", "url": "https://legacy.example.com/sse" },
        "current": { "type": "streamable-http", "url": "https://example.com/mcp" },
    }));

    assert_eq!(parsed.servers.len(), 1);
    assert_eq!(parsed.servers[0].name, "current");
    let diagnostic = &parsed.diagnostics[0];
    assert_eq!(
        diagnostic.code,
        PluginDiagnosticCode::McpUnsupportedTransport
    );
    assert_eq!(diagnostic.component.as_deref(), Some("legacy-events"));
}

#[test]
fn invalid_server_entries() {
    // (case, entry, expected code)
    let cases: Vec<(&str, Value, PluginDiagnosticCode)> = vec![
        (
            "not an object",
            json!("server"),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "missing type",
            json!({ "command": "server" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "unknown type",
            json!({ "type": "websocket", "url": "https://example.com" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio missing command",
            json!({ "type": "stdio" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio with a field from another variant",
            json!({ "type": "stdio", "command": "server", "url": "https://example.com" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio with an unknown field",
            json!({ "type": "stdio", "command": "server", "timeout": 30 }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio command is a shell string",
            json!({ "type": "stdio", "command": "node server.js" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio command is an absolute path",
            json!({ "type": "stdio", "command": "/usr/local/bin/server" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio command escapes with a relative path",
            json!({ "type": "stdio", "command": "../bin/server" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio command uses a placeholder",
            json!({ "type": "stdio", "command": "${PLUGIN_ROOT}/bin/server" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio env defines PLUGIN_ROOT",
            json!({ "type": "stdio", "command": "server", "env": { "PLUGIN_ROOT": "/tmp" } }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio env defines PLUGIN_DATA",
            json!({ "type": "stdio", "command": "server", "env": { "PLUGIN_DATA": "/tmp" } }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio env value is not a string",
            json!({ "type": "stdio", "command": "server", "env": { "PORT": 8080 } }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio args is not an array",
            json!({ "type": "stdio", "command": "server", "args": "--flag" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio cwd is not plugin-relative",
            json!({ "type": "stdio", "command": "server", "cwd": "data" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio cwd is absolute",
            json!({ "type": "stdio", "command": "server", "cwd": "/tmp" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio cwd uses an unknown placeholder",
            json!({ "type": "stdio", "command": "server", "cwd": "${HOME}/data" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "stdio cwd placeholder is not followed by a separator",
            json!({ "type": "stdio", "command": "server", "cwd": "${PLUGIN_ROOT}x" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "http missing url",
            json!({ "type": "streamable-http" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "http url is relative",
            json!({ "type": "streamable-http", "url": "/mcp" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "http url has a non-http scheme",
            json!({ "type": "streamable-http", "url": "ftp://example.com/mcp" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "http url has user information",
            json!({ "type": "streamable-http", "url": "https://user:pass@example.com/mcp" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "http url has a fragment",
            json!({ "type": "streamable-http", "url": "https://example.com/mcp#section" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "plain http to a non-loopback host",
            json!({ "type": "streamable-http", "url": "http://example.com/mcp" }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "duplicate header names under different casing",
            json!({
                "type": "streamable-http",
                "url": "https://example.com/mcp",
                "headers": { "X-Tenant": "a", "x-tenant": "b" },
            }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "invalid header name",
            json!({
                "type": "streamable-http",
                "url": "https://example.com/mcp",
                "headers": { "X Tenant": "a" },
            }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
        (
            "header value contains a newline",
            json!({
                "type": "streamable-http",
                "url": "https://example.com/mcp",
                "headers": { "X-Tenant": "a\r\nX-Injected: b" },
            }),
            PluginDiagnosticCode::McpServerInvalid,
        ),
    ];

    for (case, entry, expected) in cases {
        let diagnostic = entry_diagnostic(entry)
            .unwrap_or_else(|| panic!("{case}: expected the entry to be disabled"));
        assert_eq!(diagnostic.code, expected, "{case}");
    }
}

/// §7.2.1 permits plain HTTP when the host is loopback.
#[test]
fn plain_http_is_allowed_for_loopback_hosts() {
    for url in [
        "http://localhost:3000/mcp",
        "http://LOCALHOST:3000/mcp",
        "http://127.0.0.1:3000/mcp",
        "http://[::1]:3000/mcp",
    ] {
        let parsed = parse(json!({ "s": { "type": "streamable-http", "url": url } }));
        assert!(
            parsed.diagnostics.is_empty(),
            "'{url}' should be accepted, got {:?}",
            parsed.diagnostics
        );
    }
}

/// A bare name and a `./`-relative path are the two accepted `command` forms.
#[test]
fn accepted_command_tokens() {
    for command in ["server", "npx", "./bin/server", "./server"] {
        let parsed = parse(json!({ "s": { "type": "stdio", "command": command } }));
        assert!(
            parsed.diagnostics.is_empty(),
            "'{command}' should be accepted, got {:?}",
            parsed.diagnostics
        );
    }
}

/// All three accepted `cwd` forms, including the bare-placeholder spellings.
#[test]
fn accepted_cwd_forms() {
    for cwd in [
        "./data",
        "${PLUGIN_ROOT}",
        "${PLUGIN_ROOT}/data",
        "${PLUGIN_DATA}",
        "${PLUGIN_DATA}/cache",
    ] {
        let parsed = parse(json!({ "s": { "type": "stdio", "command": "server", "cwd": cwd } }));
        assert!(
            parsed.diagnostics.is_empty(),
            "'{cwd}' should be accepted, got {:?}",
            parsed.diagnostics
        );
    }
}
