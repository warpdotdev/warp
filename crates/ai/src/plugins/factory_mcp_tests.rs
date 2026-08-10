use std::path::Path;

use serde_json::{Value, json};

use super::*;
use crate::plugins::mcp::MCP_SCHEMA_1_0_0;

const FILE_PATH: &str = "/checkout/factory/agents/release/mcp.json";

fn parse(servers: Value) -> FactoryMcpFile {
    let content = json!({ "$schema": FACTORY_MCP_SCHEMA_1_0_0, "mcpServers": servers }).to_string();
    parse_factory_mcp_file(Path::new(FILE_PATH), &content).expect("the file is usable")
}

/// The behaviour the client/server contract turns on: managed entries belong to Factoryfile sync,
/// so the client recognizes them, creates nothing, and does not treat them as an error.
#[test]
fn managed_entries_are_recognized_and_ignored_while_ordinary_entries_load() {
    let file = parse(json!({
        "search": { "type": "managed", "warpId": "00000000-0000-0000-0000-000000000000" },
        "lint": { "type": "stdio", "command": "./bin/lint-server", "args": ["--mode", "factory"] },
        "issues": { "type": "streamable-http", "url": "https://mcp.example.com/issues" },
    }));

    assert!(
        file.diagnostics.is_empty(),
        "a managed entry is not an error"
    );
    assert_eq!(file.ignored_managed, vec!["search".to_owned()]);
    let names: Vec<&str> = file
        .servers
        .iter()
        .map(|server| server.name.as_str())
        .collect();
    assert_eq!(names, vec!["issues", "lint"]);
    assert!(
        file.servers
            .iter()
            .all(|server| server.entry.is_client_owned())
    );
}

/// A file with nothing but managed entries is valid and simply produces no installations.
#[test]
fn a_managed_only_file_produces_no_client_installations() {
    let file = parse(json!({
        "search": { "type": "managed", "warpId": "11111111-1111-1111-1111-111111111111" },
    }));
    assert!(file.servers.is_empty());
    assert!(file.diagnostics.is_empty());
    assert_eq!(file.ignored_managed.len(), 1);
}

/// Relative paths resolve against the entity directory that holds the file, not a plugin root.
#[test]
fn the_entity_directory_is_the_base_for_relative_paths() {
    let file = parse(json!({
        "lint": { "type": "stdio", "command": "./bin/lint-server", "cwd": "./" },
    }));
    assert_eq!(
        file.entity_dir,
        Path::new("/checkout/factory/agents/release")
    );
}

/// §61: the Agent Plugins schema at a Factory entity location is invalid, with a message that
/// explains where Agent Plugins MCP actually belongs.
#[test]
fn the_agent_plugins_schema_is_rejected_at_a_factory_location() {
    let content = json!({ "$schema": MCP_SCHEMA_1_0_0, "mcpServers": {} }).to_string();
    let diagnostic = parse_factory_mcp_file(Path::new(FILE_PATH), &content).unwrap_err();
    assert_eq!(
        diagnostic.code,
        PluginDiagnosticCode::FactoryMcpAgentPluginsSchema
    );
    assert!(diagnostic.reason.contains("plugin package"));
}

#[test]
fn unusable_files_are_rejected_whole() {
    let cases: Vec<(&str, String)> = vec![
        ("not JSON", "{".to_owned()),
        ("not an object", "[]".to_owned()),
        ("missing $schema", json!({ "mcpServers": {} }).to_string()),
        (
            "unknown $schema",
            json!({ "$schema": "https://example.com/x.json", "mcpServers": {} }).to_string(),
        ),
        (
            "missing mcpServers",
            json!({ "$schema": FACTORY_MCP_SCHEMA_1_0_0 }).to_string(),
        ),
        (
            "extra top-level field",
            json!({ "$schema": FACTORY_MCP_SCHEMA_1_0_0, "mcpServers": {}, "extra": 1 })
                .to_string(),
        ),
    ];
    for (case, content) in cases {
        let diagnostic = parse_factory_mcp_file(Path::new(FILE_PATH), &content)
            .err()
            .unwrap_or_else(|| panic!("{case}: expected the file to be rejected"));
        assert!(
            matches!(
                diagnostic.code,
                PluginDiagnosticCode::FactoryMcpInvalid
                    | PluginDiagnosticCode::FactoryMcpAgentPluginsSchema
            ),
            "{case}: unexpected code {:?}",
            diagnostic.code
        );
    }
}

#[test]
fn invalid_entries_are_isolated() {
    let file = parse(json!({
        "good": { "type": "stdio", "command": "./bin/server" },
        "managed-without-id": { "type": "managed" },
        "managed-with-extra": {
            "type": "managed",
            "warpId": "00000000-0000-0000-0000-000000000000",
            "command": "./bin/sneaky",
        },
        "unknown-type": { "type": "websocket", "url": "wss://example.com" },
    }));

    let names: Vec<&str> = file
        .servers
        .iter()
        .map(|server| server.name.as_str())
        .collect();
    assert_eq!(names, vec!["good"]);
    assert!(file.ignored_managed.is_empty());
    assert_eq!(file.diagnostics.len(), 3);
    assert!(
        file.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == PluginDiagnosticCode::FactoryMcpEntryInvalid)
    );
}

/// §55: a Factory file defines no plugin placeholders, so using one is an authoring error rather
/// than something silently passed through as a literal.
#[test]
fn plugin_placeholders_are_rejected_in_a_factory_file() {
    for entry in [
        json!({ "type": "stdio", "command": "${PLUGIN_ROOT}/bin/server" }),
        json!({ "type": "stdio", "command": "./s", "args": ["${PLUGIN_DATA}/db"] }),
        json!({ "type": "stdio", "command": "./s", "env": { "DIR": "${PLUGIN_DATA}" } }),
        json!({ "type": "stdio", "command": "./s", "cwd": "${PLUGIN_ROOT}" }),
    ] {
        let file = parse(json!({ "entry": entry }));
        assert!(
            file.servers.is_empty(),
            "the entry should have been disabled"
        );
        assert_eq!(
            file.diagnostics[0].code,
            PluginDiagnosticCode::FactoryMcpEntryInvalid
        );
        assert!(file.diagnostics[0].reason.contains("plugin package"));
    }
}

#[test]
fn a_stdio_entry_parses_its_fields() {
    let file = parse(json!({
        "lint": {
            "type": "stdio",
            "command": "./bin/lint-server",
            "args": ["--mode", "factory"],
            "env": { "LEVEL": "strict" },
            "cwd": "./",
        },
    }));

    let FactoryMcpEntry::Stdio {
        command,
        args,
        env,
        cwd,
    } = &file.servers[0].entry
    else {
        panic!("expected a stdio entry");
    };
    assert_eq!(command, "./bin/lint-server");
    assert_eq!(args, &["--mode".to_owned(), "factory".to_owned()]);
    assert_eq!(env["LEVEL"], "strict");
    assert_eq!(cwd.as_deref(), Some("./"));
}
