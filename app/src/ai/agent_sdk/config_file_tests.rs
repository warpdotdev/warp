#![cfg(not(target_family = "wasm"))]

use std::io::Write as _;
use std::path::PathBuf;

use ai::hooks::{ExplicitHookSource, ExplicitSourceScope, HookFormat};
use serde_json::json;
use warp_cli::agent::HooksOnAsk;
use warp_cli::mcp::MCPSpec;
use warp_core::features::FeatureFlag;

use crate::ai::ambient_agents::AgentConfigSnapshot;
use crate::ai::blocklist::hooks::RunSurface;

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
        experimental: None,
        name: Some("cli-name".to_string()),
        environment_id: None,
        runner_id: None,
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
        additional_source_repos: None,
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
fn well_known_warp_id_converts_to_well_known_spec() {
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(true);
    let contents = json!({
        "mcp_servers": {
            "linear": { "warp_id": "linear" }
        }
    })
    .to_string();

    let file = write_temp(".json", &contents);
    let loaded = super::load_config_file(file.path()).unwrap();

    let map = loaded.file.mcp_servers.as_ref().unwrap();
    let specs = super::mcp_specs_from_mcp_servers(map).unwrap();

    assert_eq!(specs.len(), 1);
    assert!(
        matches!(&specs[0], MCPSpec::WellKnown(id) if id == "linear"),
        "well-known warp_id must become MCPSpec::WellKnown, got {specs:?}"
    );
}

#[test]
fn any_non_uuid_warp_id_becomes_well_known_spec() {
    // The server owns the set of recognized ids: the client forwards any
    // non-UUID warp_id for resolution instead of rejecting it, so new ids can
    // be introduced server-side without a client change.
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(true);
    let map = serde_json::Map::from_iter([(
        "future".to_string(),
        json!({ "warp_id": "some-future-integration" }),
    )]);

    let specs = super::mcp_specs_from_mcp_servers(&map).unwrap();
    assert_eq!(specs.len(), 1);
    assert!(matches!(&specs[0], MCPSpec::WellKnown(id) if id == "some-future-integration"));
}

#[test]
fn empty_warp_id_is_rejected() {
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(true);
    let map = serde_json::Map::from_iter([("bogus".to_string(), json!({ "warp_id": "" }))]);

    let err = super::mcp_specs_from_mcp_servers(&map).unwrap_err();
    assert!(format!("{err:#}").contains("must be non-empty"));
}

#[test]
fn non_uuid_warp_id_is_rejected_when_flag_disabled() {
    let _flag = FeatureFlag::WellKnownMcpIds.override_enabled(false);
    let map = serde_json::Map::from_iter([("linear".to_string(), json!({ "warp_id": "linear" }))]);

    let err = super::mcp_specs_from_mcp_servers(&map).unwrap_err();
    assert!(format!("{err:#}").contains("must be a UUID"));
}

#[test]
fn hook_entries_resolve_against_the_config_files_directory() {
    let _flag = FeatureFlag::AgentHooks.override_enabled(true);
    let dir = tempfile::TempDir::new().unwrap();
    let config = dir.path().join("agent.yaml");
    std::fs::write(
        &config,
        "hooks:\n  - path: ./hooks\n  - path: /abs/hooks\n    format: cursor\n    scope: factory_wide\nhooks_strict: true\nhooks_on_ask: fail_run\n",
    )
    .unwrap();

    let loaded = super::load_config_file_for_local_run(&config).unwrap();

    let config_dir = dunce::canonicalize(dir.path()).unwrap();
    assert_eq!(
        loaded.hooks.sources,
        vec![
            ExplicitHookSource {
                path: config_dir.join("./hooks").join("hooks.json"),
                format: HookFormat::Cursor,
                scope: ExplicitSourceScope::Run,
            },
            ExplicitHookSource {
                path: PathBuf::from("/abs/hooks/hooks.json"),
                format: HookFormat::Cursor,
                scope: ExplicitSourceScope::FactoryWide,
            },
        ]
    );
    assert_eq!(loaded.hooks.strict, Some(true));
    assert_eq!(loaded.hooks.on_ask, Some(HooksOnAsk::FailRun));
}

#[test]
fn unknown_hook_format_scope_and_on_ask_are_rejected_with_the_supported_values() {
    let _flag = FeatureFlag::AgentHooks.override_enabled(true);
    for (contents, expected) in [
        (
            json!({ "hooks": [{ "path": "h", "format": "claude" }] }),
            "hooks[0].format must be one of: cursor (got 'claude')",
        ),
        (
            json!({ "hooks": [{ "path": "h", "scope": "team" }] }),
            "hooks[0].scope must be one of: run, factory_agent, factory_wide (got 'team')",
        ),
        (
            json!({ "hooks": [{ "path": "  " }] }),
            "hooks[0].path must not be empty",
        ),
        (
            json!({ "hooks_on_ask": "ask" }),
            "hooks_on_ask must be one of: deny, fail_run (got 'ask')",
        ),
    ] {
        let file = write_temp(".json", &contents.to_string());
        let err = format!(
            "{:#}",
            super::load_config_file_for_local_run(file.path()).unwrap_err()
        );
        assert!(err.contains(expected), "{err}");
    }
}

#[test]
fn unknown_keys_in_a_hook_entry_are_rejected() {
    let _flag = FeatureFlag::AgentHooks.override_enabled(true);
    let file = write_temp(
        ".json",
        &json!({ "hooks": [{ "path": "h", "timeout": 5 }] }).to_string(),
    );
    assert!(super::load_config_file_for_local_run(file.path()).is_err());
}

#[test]
fn supported_keys_list_the_hook_keys() {
    let context = super::supported_keys_context();
    for key in ["hooks", "hooks_strict", "hooks_on_ask"] {
        assert!(context.contains(key), "{context}");
    }
}

#[test]
fn hook_settings_are_rejected_outside_a_local_run_and_when_hooks_are_off() {
    let file = write_temp(".json", &json!({ "hooks_strict": true }).to_string());
    {
        let _flag = FeatureFlag::AgentHooks.override_enabled(true);
        let err = format!("{:#}", super::load_config_file(file.path()).unwrap_err());
        assert!(err.contains("apply only to `agent run`"), "{err}");
    }
    let _flag = FeatureFlag::AgentHooks.override_enabled(false);
    let err = format!(
        "{:#}",
        super::load_config_file_for_local_run(file.path()).unwrap_err()
    );
    assert!(err.contains("agent hooks are not enabled"), "{err}");
}

#[test]
fn command_line_hook_settings_win_over_the_file() {
    let _flag = FeatureFlag::AgentHooks.override_enabled(true);
    let file = write_temp(
        ".json",
        &json!({ "hooks": [{ "path": "/h" }], "hooks_strict": false, "hooks_on_ask": "fail_run" })
            .to_string(),
    );
    let loaded = super::load_config_file_for_local_run(file.path()).unwrap();

    let from_file = super::merge_hook_settings(
        Some(&loaded),
        false,
        None,
        Vec::new(),
        RunSurface::CommandLine,
    );
    assert!(!from_file.strict);
    assert_eq!(from_file.on_ask, HooksOnAsk::FailRun);
    assert_eq!(from_file.configured_sources.len(), 1);

    let from_cli = super::merge_hook_settings(
        Some(&loaded),
        true,
        Some(HooksOnAsk::Deny),
        vec![PathBuf::from("extra")],
        RunSurface::Oz,
    );
    assert!(from_cli.strict);
    assert_eq!(from_cli.on_ask, HooksOnAsk::Deny);
    assert_eq!(from_cli.extra_dirs, vec![PathBuf::from("extra")]);
    assert_eq!(from_cli.surface, RunSurface::Oz);

    let defaults = super::merge_hook_settings(None, false, None, Vec::new(), RunSurface::Oz);
    assert!(!defaults.strict);
    assert_eq!(defaults.on_ask, HooksOnAsk::Deny);
    assert!(defaults.configured_sources.is_empty());
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
