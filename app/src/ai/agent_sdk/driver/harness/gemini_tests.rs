use std::fs;

use serde_json::Value;
use tempfile::TempDir;

use super::*;

#[test]
fn prepare_gemini_settings_creates_file_with_api_key_auth() {
    let tmp = TempDir::new().unwrap();
    let settings_path = tmp.path().join("settings.json");

    prepare_gemini_settings(&settings_path, false, &HashMap::new()).unwrap();

    let settings: Value = serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
    assert_eq!(
        settings["security"]["auth"]["selectedType"],
        Value::String("gemini-api-key".to_owned()),
    );
}

#[test]
fn prepare_gemini_settings_preserves_unrelated_keys() {
    let tmp = TempDir::new().unwrap();
    let settings_path = tmp.path().join("settings.json");
    fs::write(
        &settings_path,
        r#"{
            "ui": {"theme": "dark"},
            "security": {
                "folderTrust": {"enabled": true},
                "auth": {"enforcedType": "vertex-ai"}
            }
        }"#,
    )
    .unwrap();

    prepare_gemini_settings(&settings_path, false, &HashMap::new()).unwrap();

    let settings: Value = serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
    assert_eq!(settings["ui"]["theme"], "dark");
    assert_eq!(settings["security"]["folderTrust"]["enabled"], true);
    // Sibling auth fields survive, and selectedType is set alongside them.
    assert_eq!(settings["security"]["auth"]["enforcedType"], "vertex-ai");
    assert_eq!(
        settings["security"]["auth"]["selectedType"],
        "gemini-api-key",
    );
}

#[test]
fn prepare_gemini_settings_surfaces_malformed_json_as_error() {
    let tmp = TempDir::new().unwrap();
    let settings_path = tmp.path().join("settings.json");
    // `security` typed as a string instead of an object is a parse error;
    // we prefer surfacing that to silently rewriting user-owned state.
    fs::write(
        &settings_path,
        r#"{"ui":{"theme":"dark"},"security":"broken"}"#,
    )
    .unwrap();

    assert!(prepare_gemini_settings(&settings_path, false, &HashMap::new()).is_err());
}

#[test]
fn prepare_gemini_trusted_folders_creates_file_with_working_dir() {
    let tmp = TempDir::new().unwrap();
    let trusted_path = tmp.path().join("trustedFolders.json");
    let working_dir = tmp.path().join("workspace/project");

    prepare_gemini_trusted_folders(&trusted_path, &working_dir).unwrap();

    let trusted: Value = serde_json::from_slice(&fs::read(trusted_path).unwrap()).unwrap();
    let working_dir_key = working_dir.to_string_lossy().to_string();
    assert_eq!(trusted[working_dir_key], "TRUST_FOLDER");
}

#[test]
fn prepare_gemini_trusted_folders_preserves_existing_entries() {
    let tmp = TempDir::new().unwrap();
    let trusted_path = tmp.path().join("trustedFolders.json");
    fs::write(
        &trusted_path,
        r#"{"/other/project":"TRUST_PARENT","/do/not/trust":"DO_NOT_TRUST"}"#,
    )
    .unwrap();
    let working_dir = tmp.path().join("workspace/project");

    prepare_gemini_trusted_folders(&trusted_path, &working_dir).unwrap();

    let trusted: Value = serde_json::from_slice(&fs::read(trusted_path).unwrap()).unwrap();
    assert_eq!(trusted["/other/project"], "TRUST_PARENT");
    assert_eq!(trusted["/do/not/trust"], "DO_NOT_TRUST");
    let working_dir_key = working_dir.to_string_lossy().to_string();
    assert_eq!(trusted[working_dir_key], "TRUST_FOLDER");
}

#[test]
fn prepare_gemini_settings_adds_context_file_name_when_system_prompt_present() {
    let tmp = TempDir::new().unwrap();
    let settings_path = tmp.path().join("settings.json");

    prepare_gemini_settings(&settings_path, true, &HashMap::new()).unwrap();

    let settings: Value = serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
    assert_eq!(
        settings["context"]["fileName"],
        Value::Array(vec![Value::String("OZ_SYSTEM_PROMPT.md".to_owned())]),
    );
}

#[test]
fn prepare_gemini_settings_appends_to_existing_context_file_name() {
    let tmp = TempDir::new().unwrap();
    let settings_path = tmp.path().join("settings.json");
    fs::write(
        &settings_path,
        r#"{"context":{"fileName":["existing.md"]}}"#,
    )
    .unwrap();

    prepare_gemini_settings(&settings_path, true, &HashMap::new()).unwrap();

    let settings: Value = serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
    let file_names: Vec<String> = settings["context"]["fileName"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(file_names, vec!["existing.md", "OZ_SYSTEM_PROMPT.md"]);
}

#[test]
fn prepare_gemini_settings_does_not_duplicate_system_prompt_file_name() {
    let tmp = TempDir::new().unwrap();
    let settings_path = tmp.path().join("settings.json");
    fs::write(
        &settings_path,
        r#"{"context":{"fileName":["OZ_SYSTEM_PROMPT.md"]}}"#,
    )
    .unwrap();

    prepare_gemini_settings(&settings_path, true, &HashMap::new()).unwrap();

    let settings: Value = serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
    let file_names: Vec<String> = settings["context"]["fileName"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(file_names, vec!["OZ_SYSTEM_PROMPT.md"]);
}

#[test]
fn prepare_gemini_settings_omits_context_when_no_system_prompt() {
    let tmp = TempDir::new().unwrap();
    let settings_path = tmp.path().join("settings.json");

    prepare_gemini_settings(&settings_path, false, &HashMap::new()).unwrap();

    let settings: Value = serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
    assert!(settings.get("context").is_none());
}

#[test]
fn prepare_gemini_settings_merges_remote_and_command_mcp_servers() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("settings.json");
    fs::write(
        &path,
        r#"{
            "ui": {"theme": "dark"},
            "mcpServers": {
                "existing": {"command": "custom", "timeout": 9000},
                "github": {"url": "https://old.example/mcp", "disabled": true}
            }
        }"#,
    )
    .unwrap();
    let servers = HashMap::from([
        (
            "github".to_owned(),
            JSONMCPServer {
                transport_type: JSONTransportType::SSEServer {
                    url: "https://proxy.example/mcp".to_owned(),
                    headers: HashMap::from([
                        (
                            "Authorization".to_owned(),
                            "Bearer test-proxy-token".to_owned(),
                        ),
                        ("X-Warp-Run-ID".to_owned(), "test-run".to_owned()),
                    ]),
                },
            },
        ),
        (
            "local".to_owned(),
            JSONMCPServer {
                transport_type: JSONTransportType::CLIServer {
                    command: "npx".to_owned(),
                    args: vec!["example-mcp".to_owned()],
                    env: HashMap::from([("MODE".to_owned(), "test".to_owned())]),
                    working_directory: Some("/workspace".to_owned()),
                },
            },
        ),
    ]);

    prepare_gemini_settings(&path, true, &servers).unwrap();
    let settings: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(settings["ui"]["theme"], "dark");
    assert_eq!(
        settings["mcpServers"]["existing"],
        json!({"command": "custom", "timeout": 9000})
    );
    assert_eq!(
        settings["mcpServers"]["github"],
        json!({
            "httpUrl": "https://proxy.example/mcp",
            "headers": {
                "Authorization": "Bearer test-proxy-token",
                "X-Warp-Run-ID": "test-run"
            }
        })
    );
    assert_eq!(
        settings["mcpServers"]["local"],
        json!({"command": "npx", "args": ["example-mcp"], "env": {"MODE": "test"}, "cwd": "/workspace"})
    );

    prepare_gemini_settings(&path, true, &servers).unwrap();
    let repeated: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(settings, repeated);
}

#[test]
fn prepare_gemini_settings_preserves_mcp_servers_without_terminal_registrations() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("settings.json");
    let existing = json!({"existing": {"url": "https://existing.example/sse", "timeout": 9000}});
    fs::write(&path, json!({"mcpServers": existing}).to_string()).unwrap();

    prepare_gemini_settings(&path, false, &HashMap::new()).unwrap();

    let settings: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(settings["mcpServers"], existing);
}
