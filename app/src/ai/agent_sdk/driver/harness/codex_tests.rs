use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::sync::Arc;

use serde_json::Value;
use tempfile::TempDir;
use uuid::Uuid;

use super::super::codex_transcript::CodexTranscriptEnvelope;
use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::server::server_api::harness_support::MockHarnessSupportClient;

#[test]
fn prepare_codex_auth_writes_fresh_file_with_api_key_mode() {
    let tmp = TempDir::new().unwrap();
    let auth_path = tmp.path().join(".codex/auth.json");

    prepare_codex_auth(&auth_path, "sk-test-key").unwrap();

    let auth: Value = serde_json::from_slice(&fs::read(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "sk-test-key");
    assert_eq!(auth["auth_mode"], "apikey");
}

fn factory_mcp_server(token: &str) -> JSONMCPServer {
    factory_mcp_server_at("https://app.warp.dev/api/v1/mcp/factory", token)
}

fn factory_mcp_server_at(url: &str, token: &str) -> JSONMCPServer {
    JSONMCPServer {
        transport_type: JSONTransportType::SSEServer {
            url: url.to_string(),
            headers: HashMap::from([("Authorization".to_string(), format!("Bearer {token}"))]),
        },
    }
}

#[test]
#[serial_test::serial]
fn codex_runs_isolate_builtin_and_explicit_factory_mcp_config() {
    let tmp = TempDir::new().unwrap();
    let persistent_home = tmp.path().join("persistent-codex-home");
    let builtin_working_dir = tmp.path().join("builtin-workspace");
    let explicit_working_dir = tmp.path().join("explicit-workspace");
    let persistent_config = r#"
[mcp_servers.warp-factory]
command = "persistent-command"
args = ["--persistent"]
env = { TOKEN = "persistent-token" }
cwd = "/persistent/cwd"
enabled = false

[mcp_servers.user-server]
command = "user-command"
"#;
    let persistent_auth = r#"{"auth_mode":"Chatgpt","tokens":{"access_token":"user-token"}}"#;
    fs::create_dir_all(persistent_home.join("plugins/example-plugin")).unwrap();
    fs::create_dir_all(&builtin_working_dir).unwrap();
    fs::create_dir_all(&explicit_working_dir).unwrap();
    fs::write(persistent_home.join("config.toml"), persistent_config).unwrap();
    fs::write(persistent_home.join("auth.json"), persistent_auth).unwrap();
    fs::write(
        persistent_home.join("plugins/example-plugin/plugin.json"),
        "{}",
    )
    .unwrap();

    let previous_codex_home = std::env::var_os(CODEX_HOME_ENV);
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(CODEX_HOME_ENV, &persistent_home) };

    let (builtin_home, explicit_home) = std::thread::scope(|scope| {
        let builtin = scope.spawn(|| {
            prepare_codex_environment_config(
                &builtin_working_dir,
                &builtin_working_dir,
                None,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::from([(
                    FACTORY_MCP_SERVER_NAME.to_string(),
                    factory_mcp_server("parent-token"),
                )]),
                None,
            )
            .unwrap()
        });
        let explicit = scope.spawn(|| {
            prepare_codex_environment_config(
                &explicit_working_dir,
                &explicit_working_dir,
                None,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::from([(
                    FACTORY_MCP_SERVER_NAME.to_string(),
                    factory_mcp_server_at("https://user.example.com/factory", "explicit-token"),
                )]),
                None,
            )
            .unwrap()
        });
        (builtin.join().unwrap(), explicit.join().unwrap())
    });

    match previous_codex_home {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(value) => unsafe { std::env::set_var(CODEX_HOME_ENV, value) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var(CODEX_HOME_ENV) },
    }

    let builtin_path = builtin_home.path().to_path_buf();
    let explicit_path = explicit_home.path().to_path_buf();
    assert_ne!(builtin_path, explicit_path);
    let builtin_config = read_codex_config(&builtin_path.join("config.toml"));
    let builtin_factory = &builtin_config["mcp_servers"][FACTORY_MCP_SERVER_NAME];
    assert_eq!(
        builtin_factory["url"].as_str(),
        Some("https://app.warp.dev/api/v1/mcp/factory")
    );
    assert_eq!(
        builtin_factory["http_headers"]["Authorization"].as_str(),
        Some("Bearer parent-token")
    );
    for stale_key in ["command", "args", "env", "cwd", "enabled"] {
        assert!(builtin_factory.get(stale_key).is_none());
    }

    let explicit_config = read_codex_config(&explicit_path.join("config.toml"));
    let explicit_factory = &explicit_config["mcp_servers"][FACTORY_MCP_SERVER_NAME];
    assert_eq!(
        explicit_factory["url"].as_str(),
        Some("https://user.example.com/factory")
    );
    assert_eq!(
        explicit_factory["http_headers"]["Authorization"].as_str(),
        Some("Bearer explicit-token")
    );
    for stale_key in ["command", "args", "env", "cwd", "enabled"] {
        assert!(explicit_factory.get(stale_key).is_none());
    }
    assert_eq!(
        fs::read_to_string(persistent_home.join("config.toml")).unwrap(),
        persistent_config
    );
    assert_eq!(
        fs::read_to_string(persistent_home.join("auth.json")).unwrap(),
        persistent_auth
    );
    assert!(
        builtin_path
            .join("plugins/example-plugin/plugin.json")
            .exists()
    );
    assert!(
        explicit_path
            .join("plugins/example-plugin/plugin.json")
            .exists()
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        assert_eq!(
            fs::metadata(&builtin_path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(builtin_path.join("config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    drop(builtin_home);
    assert!(!builtin_path.exists());
    assert!(explicit_path.exists());
    drop(explicit_home);
    assert!(!explicit_path.exists());
}

#[cfg(unix)]
#[test]
fn factory_mcp_config_is_written_with_0600_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    fs::write(&config_path, "").unwrap();
    fs::set_permissions(&config_path, fs::Permissions::from_mode(0o644)).unwrap();

    prepare_codex_config_toml(
        &config_path,
        &working_dir,
        &HashMap::from([(
            FACTORY_MCP_SERVER_NAME.to_string(),
            factory_mcp_server("parent-token"),
        )]),
        None,
        None,
    )
    .unwrap();

    let mode = fs::metadata(&config_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn prepare_codex_auth_preserves_unrelated_fields() {
    let tmp = TempDir::new().unwrap();
    let auth_path = tmp.path().join("auth.json");
    fs::write(
        &auth_path,
        r#"{"tokens":{"access_token":"tok"},"last_refresh":"2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();

    prepare_codex_auth(&auth_path, "sk-new-key").unwrap();

    let auth: Value = serde_json::from_slice(&fs::read(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "sk-new-key");
    assert_eq!(auth["auth_mode"], "apikey");
    assert_eq!(auth["tokens"]["access_token"], "tok");
    assert_eq!(auth["last_refresh"], "2026-01-01T00:00:00Z");
}

#[test]
fn prepare_codex_auth_does_not_overwrite_existing_auth_mode() {
    let tmp = TempDir::new().unwrap();
    let auth_path = tmp.path().join("auth.json");
    fs::write(&auth_path, r#"{"auth_mode":"Chatgpt"}"#).unwrap();

    prepare_codex_auth(&auth_path, "sk-new-key").unwrap();

    let auth: Value = serde_json::from_slice(&fs::read(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["auth_mode"], "Chatgpt");
    assert_eq!(auth["OPENAI_API_KEY"], "sk-new-key");
}

#[test]
fn prepare_codex_auth_overwrites_stale_openai_api_key() {
    let tmp = TempDir::new().unwrap();
    let auth_path = tmp.path().join("auth.json");
    fs::write(
        &auth_path,
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-old"}"#,
    )
    .unwrap();

    prepare_codex_auth(&auth_path, "sk-new").unwrap();

    let auth: Value = serde_json::from_slice(&fs::read(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "sk-new");
}

#[cfg(unix)]
#[test]
fn prepare_codex_auth_writes_with_0600_perms() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let auth_path = tmp.path().join(".codex/auth.json");

    prepare_codex_auth(&auth_path, "sk-test-key").unwrap();

    let mode = fs::metadata(&auth_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn resolve_openai_api_key_returns_value_from_resolved_map() {
    let resolved = HashMap::from([(
        OsString::from("OPENAI_API_KEY"),
        OsString::from("sk-from-secret"),
    )]);
    assert_eq!(
        resolve_openai_api_key(&resolved).as_deref(),
        Some("sk-from-secret")
    );
}

#[test]
#[serial_test::serial]
fn resolve_openai_api_key_falls_back_to_env_var() {
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(OPENAI_API_KEY_ENV, "sk-from-env") };

    let result = resolve_openai_api_key(&HashMap::new());

    match prev {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(v) => unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) },
    }
    assert_eq!(result.as_deref(), Some("sk-from-env"));
}

#[test]
#[serial_test::serial]
fn resolve_openai_api_key_returns_none_when_map_and_env_empty() {
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) };

    let result = resolve_openai_api_key(&HashMap::new());

    if let Some(v) = prev {
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) };
    }
    assert_eq!(result, None);
}

#[test]
#[serial_test::serial]
fn resolve_openai_api_key_prefers_env_over_resolved_map() {
    // Worker-injected env var wins over the resolved secret map because
    // build_secret_env_vars skips secrets that collide with process env.
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(OPENAI_API_KEY_ENV, "sk-from-env") };
    let resolved = HashMap::from([(
        OsString::from("OPENAI_API_KEY"),
        OsString::from("sk-from-secret"),
    )]);

    let result = resolve_openai_api_key(&resolved);

    match prev {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(v) => unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) },
    }
    assert_eq!(result.as_deref(), Some("sk-from-env"));
}

#[test]
#[serial_test::serial]
fn resolve_openai_api_key_uses_resolved_map_when_env_empty() {
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(OPENAI_API_KEY_ENV, "   ") };
    let resolved = HashMap::from([(
        OsString::from("OPENAI_API_KEY"),
        OsString::from("sk-from-secret"),
    )]);

    let result = resolve_openai_api_key(&resolved);

    match prev {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(v) => unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) },
    }
    assert_eq!(result.as_deref(), Some("sk-from-secret"));
}

#[test]
#[serial_test::serial]
fn prepare_codex_environment_config_honors_codex_home() {
    let tmp = TempDir::new().unwrap();
    let codex_home = tmp.path().join("codex-home");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&codex_home).unwrap();
    fs::create_dir_all(&working_dir).unwrap();
    fs::write(
        codex_home.join(CODEX_CONFIG_TOML_FILE_NAME),
        "[mcp_servers.persistent]\ncommand = \"persistent-command\"\n",
    )
    .unwrap();
    let prev_codex_home = std::env::var(CODEX_HOME_ENV).ok();
    let prev_openai_api_key = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(CODEX_HOME_ENV, &codex_home) };
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) };
    let resolved = HashMap::from([(
        OsString::from(OPENAI_API_KEY_ENV),
        OsString::from("sk-from-secret"),
    )]);

    let model_config = harness_model_config("gpt-5.5", None);
    let run_home = prepare_codex_environment_config(
        &working_dir,
        &working_dir,
        Some("system prompt"),
        &resolved,
        &HashMap::new(),
        &HashMap::new(),
        Some(&model_config),
    )
    .unwrap();

    match prev_codex_home {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(v) => unsafe { std::env::set_var(CODEX_HOME_ENV, v) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var(CODEX_HOME_ENV) },
    }
    match prev_openai_api_key {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(v) => unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) },
    }
    let auth: Value =
        serde_json::from_slice(&fs::read(run_home.path().join(CODEX_AUTH_FILE_NAME)).unwrap())
            .unwrap();
    assert_eq!(
        fs::read_to_string(run_home.path().join(CODEX_AGENTS_OVERRIDE_FILE_NAME)).unwrap(),
        "system prompt"
    );
    assert_eq!(auth["OPENAI_API_KEY"], "sk-from-secret");
    let cfg = read_codex_config(&run_home.path().join(CODEX_CONFIG_TOML_FILE_NAME));
    assert_eq!(cfg["model"].as_str(), Some("gpt-5.5"));
    assert_eq!(
        cfg["mcp_servers"]["persistent"]["command"].as_str(),
        Some("persistent-command")
    );
    assert!(!cfg.contains_key("openai_base_url"));
    assert!(!codex_home.join(CODEX_AUTH_FILE_NAME).exists());
    assert_eq!(
        fs::read_to_string(codex_home.join(CODEX_CONFIG_TOML_FILE_NAME)).unwrap(),
        "[mcp_servers.persistent]\ncommand = \"persistent-command\"\n"
    );
}

fn read_codex_config(path: &std::path::Path) -> toml::Table {
    let content = fs::read_to_string(path).unwrap();
    toml::from_str(&content).unwrap()
}

fn harness_model_config(model_id: &str, reasoning_level: Option<&str>) -> HarnessModelConfig {
    HarnessModelConfig {
        model_id: model_id.to_string(),
        reasoning_level: reasoning_level.map(str::to_string),
    }
}

#[test]
fn prepare_codex_config_toml_writes_fresh_config() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join(".codex/config.toml");
    let working_dir = tmp.path().join("workspace/proj");
    fs::create_dir_all(&working_dir).unwrap();

    prepare_codex_config_toml(&config_path, &working_dir, &HashMap::new(), None, None).unwrap();

    let canonical = working_dir.canonicalize().unwrap();
    let key = canonical.to_string_lossy().into_owned();
    let cfg = read_codex_config(&config_path);
    assert_eq!(cfg["check_for_update_on_startup"].as_bool(), Some(false));
    assert_eq!(
        cfg["projects"][&key]["trust_level"].as_str(),
        Some("trusted")
    );
}

#[test]
fn prepare_codex_config_toml_preserves_unrelated_keys() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    fs::write(
        &config_path,
        "model = \"gpt-5\"\n\n[projects.\"/other/path\"]\ntrust_level = \"trusted\"\n",
    )
    .unwrap();

    // Pass `None` — the `model` key is intentionally removed (managed
    // key), but unrelated keys like existing project entries are kept.
    prepare_codex_config_toml(&config_path, &working_dir, &HashMap::new(), None, None).unwrap();

    let canonical = working_dir.canonicalize().unwrap();
    let key = canonical.to_string_lossy().into_owned();
    let cfg = read_codex_config(&config_path);
    // `model` is a managed key — removed when no override is provided.
    assert!(!cfg.contains_key("model"));
    assert_eq!(
        cfg["projects"]["/other/path"]["trust_level"].as_str(),
        Some("trusted")
    );
    assert_eq!(
        cfg["projects"][&key]["trust_level"].as_str(),
        Some("trusted")
    );
}

#[test]
fn prepare_codex_config_toml_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    prepare_codex_config_toml(&config_path, &working_dir, &HashMap::new(), None, None).unwrap();
    let after_first = fs::read_to_string(&config_path).unwrap();
    prepare_codex_config_toml(&config_path, &working_dir, &HashMap::new(), None, None).unwrap();
    let after_second = fs::read_to_string(&config_path).unwrap();

    assert_eq!(after_first, after_second);
    let canonical = working_dir.canonicalize().unwrap();
    let key = canonical.to_string_lossy().into_owned();
    let cfg: toml::Table = toml::from_str(&after_second).unwrap();
    let projects = cfg["projects"].as_table().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[&key]["trust_level"].as_str(), Some("trusted"));
}

#[test]
fn prepare_codex_config_toml_upgrades_untrusted_entry() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    let canonical = working_dir.canonicalize().unwrap();
    let key = canonical.to_string_lossy().into_owned();
    // Use a TOML literal-string key ('...') so Windows backslashes in `key`
    // (e.g. `\\?\C:\...`) are not interpreted as escape sequences.
    fs::write(
        &config_path,
        format!("[projects.'{key}']\ntrust_level = \"untrusted\"\n"),
    )
    .unwrap();

    prepare_codex_config_toml(&config_path, &working_dir, &HashMap::new(), None, None).unwrap();

    let cfg = read_codex_config(&config_path);
    assert_eq!(
        cfg["projects"][&key]["trust_level"].as_str(),
        Some("trusted")
    );
}

#[test]
fn prepare_codex_config_toml_trusts_multiple_child_repos() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    let repo_a = working_dir.join("a");
    let repo_b = working_dir.join("b");
    fs::create_dir_all(repo_a.join(".git")).unwrap();
    fs::create_dir_all(repo_b.join(".git")).unwrap();

    prepare_codex_config_toml(&config_path, &working_dir, &HashMap::new(), None, None).unwrap();

    let cfg = read_codex_config(&config_path);
    let projects = cfg["projects"].as_table().unwrap();
    let canonical_a = repo_a.canonicalize().unwrap();
    let canonical_b = repo_b.canonicalize().unwrap();
    assert_eq!(
        projects[canonical_a.to_str().unwrap()]["trust_level"].as_str(),
        Some("trusted")
    );
    assert_eq!(
        projects[canonical_b.to_str().unwrap()]["trust_level"].as_str(),
        Some("trusted")
    );
}

#[test]
fn prepare_codex_config_toml_overwrites_stale_openai_base_url() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    fs::write(
        &config_path,
        "openai_base_url = \"https://api.openai.com/v1\"\n",
    )
    .unwrap();

    prepare_codex_config_toml(
        &config_path,
        &working_dir,
        &HashMap::new(),
        None,
        Some("https://custom.api.openai.com/v1"),
    )
    .unwrap();

    let cfg = read_codex_config(&config_path);
    assert_eq!(
        cfg["openai_base_url"].as_str(),
        Some("https://custom.api.openai.com/v1")
    );
}

#[test]
fn write_codex_mcp_servers_cli_server() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    let servers = HashMap::from([(
        "my-mcp".to_string(),
        JSONMCPServer {
            transport_type: JSONTransportType::CLIServer {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@some/mcp".to_string()],
                env: HashMap::from([("TOKEN".to_string(), "abc".to_string())]),
                working_directory: None,
            },
        },
    )]);
    prepare_codex_config_toml(&config_path, &working_dir, &servers, None, None).unwrap();

    let cfg = read_codex_config(&config_path);
    let mcp = &cfg["mcp_servers"]["my-mcp"];
    assert_eq!(mcp["command"].as_str(), Some("npx"));
    let args: Vec<&str> = mcp["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(args, vec!["-y", "@some/mcp"]);
    assert_eq!(mcp["env"]["TOKEN"].as_str(), Some("abc"));
}

#[test]
fn write_codex_mcp_servers_preserves_factory_mcp_auth() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    let servers = HashMap::from([(
        "warp-factory".to_string(),
        JSONMCPServer {
            transport_type: JSONTransportType::SSEServer {
                url: "https://app.warp.dev/api/v1/mcp/factory".to_string(),
                headers: HashMap::from([(
                    "Authorization".to_string(),
                    "Bearer wk-test-key".to_string(),
                )]),
            },
        },
    )]);
    prepare_codex_config_toml(&config_path, &working_dir, &servers, None, None).unwrap();

    let cfg = read_codex_config(&config_path);
    let mcp = &cfg["mcp_servers"]["warp-factory"];
    assert_eq!(
        mcp["url"].as_str(),
        Some("https://app.warp.dev/api/v1/mcp/factory")
    );
    assert_eq!(
        mcp["http_headers"]["Authorization"].as_str(),
        Some("Bearer wk-test-key")
    );
}

#[test]
fn write_codex_mcp_servers_cli_server_with_cwd() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    let servers = HashMap::from([(
        "my-mcp".to_string(),
        JSONMCPServer {
            transport_type: JSONTransportType::CLIServer {
                command: "node".to_string(),
                args: vec!["server.js".to_string()],
                env: HashMap::new(),
                working_directory: Some("/opt/mcp-server".to_string()),
            },
        },
    )]);
    prepare_codex_config_toml(&config_path, &working_dir, &servers, None, None).unwrap();

    let cfg = read_codex_config(&config_path);
    let mcp = &cfg["mcp_servers"]["my-mcp"];
    assert_eq!(mcp["command"].as_str(), Some("node"));
    assert_eq!(mcp["cwd"].as_str(), Some("/opt/mcp-server"));
}

#[test]
fn write_codex_mcp_servers_cli_server_without_cwd_omits_key() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    let servers = HashMap::from([(
        "my-mcp".to_string(),
        JSONMCPServer {
            transport_type: JSONTransportType::CLIServer {
                command: "npx".to_string(),
                args: vec![],
                env: HashMap::new(),
                working_directory: None,
            },
        },
    )]);
    prepare_codex_config_toml(&config_path, &working_dir, &servers, None, None).unwrap();

    let cfg = read_codex_config(&config_path);
    let mcp = &cfg["mcp_servers"]["my-mcp"];
    assert!(mcp.get("cwd").is_none());
}

#[test]
fn prepare_codex_config_toml_writes_model_when_specified() {
    // A non-default model id is written to the top-level `model` key so Codex pins it
    // for new sessions launched from this `~/.codex/config.toml`. Even for the
    // current target model, we stamp a self-referential migration entry so the
    // upgrade prompt is suppressed regardless of what the user selected.
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    prepare_codex_config_toml(
        &config_path,
        &working_dir,
        &HashMap::new(),
        Some(&harness_model_config("gpt-5.5", None)),
        None,
    )
    .unwrap();

    let cfg = read_codex_config(&config_path);
    assert_eq!(cfg["model"].as_str(), Some("gpt-5.5"));
    assert_eq!(
        cfg["notice"]["model_migrations"]["gpt-5.5"].as_str(),
        Some(CODEX_MODEL_MIGRATIONS_TARGET),
    );
}

#[test]
fn prepare_codex_config_toml_writes_model_migration_for_older_model() {
    // For an older model id, the migration entry maps it to the current target
    // so Codex's "choose a newer model" prompt is suppressed at session launch.
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    prepare_codex_config_toml(
        &config_path,
        &working_dir,
        &HashMap::new(),
        Some(&harness_model_config("gpt-5.2", None)),
        None,
    )
    .unwrap();

    let cfg = read_codex_config(&config_path);
    assert_eq!(cfg["model"].as_str(), Some("gpt-5.2"));
    assert_eq!(
        cfg["notice"]["model_migrations"]["gpt-5.2"].as_str(),
        Some(CODEX_MODEL_MIGRATIONS_TARGET),
    );
}

#[test]
fn prepare_codex_config_toml_skips_model_for_default_sentinel() {
    // The literal "default" sentinel means "let Codex pick its own default model";
    // we should NOT write a `model` key (or a migration entry) in that case.
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    prepare_codex_config_toml(
        &config_path,
        &working_dir,
        &HashMap::new(),
        Some(&harness_model_config("default", None)),
        None,
    )
    .unwrap();

    let cfg = read_codex_config(&config_path);
    assert!(
        cfg.get("model").is_none(),
        "`model` should not be written for the default sentinel"
    );
    assert!(
        cfg.get("notice").is_none(),
        "`[notice]` table should not be written without a pinned model id"
    );
}

#[test]
fn prepare_codex_config_toml_skips_model_when_none() {
    // No model id supplied means the user didn't pick one; we should not write a
    // `model` key or any `[notice.model_migrations]` entries.
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    prepare_codex_config_toml(&config_path, &working_dir, &HashMap::new(), None, None).unwrap();

    let cfg = read_codex_config(&config_path);
    assert!(
        cfg.get("model").is_none(),
        "`model` should not be written when no override is supplied"
    );
    assert!(
        cfg.get("notice").is_none(),
        "`[notice]` table should not be written without a pinned model id"
    );
}

#[test]
fn prepare_codex_config_toml_writes_model_reasoning_effort_when_specified() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();

    prepare_codex_config_toml(
        &config_path,
        &working_dir,
        &HashMap::new(),
        Some(&harness_model_config("gpt-5.5", Some("medium"))),
        None,
    )
    .unwrap();

    let cfg = read_codex_config(&config_path);
    assert_eq!(cfg["model"].as_str(), Some("gpt-5.5"));
    assert_eq!(cfg["model_reasoning_effort"].as_str(), Some("medium"));
}

#[test]
fn prepare_codex_config_toml_removes_stale_model_reasoning_effort_when_none() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.toml");
    let working_dir = tmp.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    fs::write(&config_path, "model_reasoning_effort = \"high\"\n").unwrap();

    prepare_codex_config_toml(&config_path, &working_dir, &HashMap::new(), None, None).unwrap();

    let cfg = read_codex_config(&config_path);
    assert!(cfg.get("model_reasoning_effort").is_none());
}

#[test]
fn find_child_git_repos_returns_only_repo_children() {
    let tmp = TempDir::new().unwrap();
    let workspace = tmp.path().join("workspace");
    let repo = workspace.join("repo");
    let other = workspace.join("other");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(&other).unwrap();

    let found = find_child_git_repos(&workspace);
    let canonical_repo = repo.canonicalize().unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].canonicalize().unwrap(), canonical_repo);
}

#[test]
fn find_child_git_repos_returns_empty_when_dir_missing() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("does-not-exist");
    assert!(find_child_git_repos(&missing).is_empty());
}

#[test]
fn codex_command_with_session_id_invokes_resume_subcommand() {
    let uuid = Uuid::new_v4();
    let cmd = codex_command(
        "codex",
        Some(&uuid),
        "/tmp/prompt.txt",
        std::path::Path::new("/tmp/run-codex-home"),
    );
    assert!(
        cmd.starts_with("CODEX_HOME='/tmp/run-codex-home' "),
        "command should set the isolated Codex home: {cmd}"
    );
    assert!(
        cmd.contains(&format!(
            "resume --dangerously-bypass-approvals-and-sandbox --dangerously-bypass-hook-trust {uuid}"
        )),
        "resume command should pass UUID to `resume`: {cmd}"
    );
    assert!(
        cmd.contains("\"$(cat '/tmp/prompt.txt')\""),
        "resume command should pipe prompt: {cmd}"
    );
}

#[test]
fn codex_command_without_session_id_bypasses_hook_trust() {
    let cmd = codex_command(
        "codex",
        None,
        "/tmp/prompt.txt",
        std::path::Path::new("/tmp/run-codex-home"),
    );
    assert!(
        cmd.starts_with("CODEX_HOME='/tmp/run-codex-home' "),
        "command should set the isolated Codex home: {cmd}"
    );
    assert!(
        cmd.contains("--dangerously-bypass-approvals-and-sandbox"),
        "command should bypass approvals and sandbox: {cmd}"
    );
    assert!(
        cmd.contains("--dangerously-bypass-hook-trust"),
        "command should bypass hook trust for driver-installed hooks: {cmd}"
    );
    assert!(
        cmd.contains("\"$(cat '/tmp/prompt.txt')\""),
        "command should pipe prompt: {cmd}"
    );
}

#[tokio::test]
async fn fetch_resume_payload_maps_404_to_resume_state_missing() {
    let mut mock = MockHarnessSupportClient::new();
    mock.expect_fetch_transcript()
        .returning(|| Err(anyhow::anyhow!("upstream returned status 404")));
    let conversation_id = AIConversationId::new();

    let result = CodexHarness
        .fetch_resume_payload(&conversation_id, Arc::new(mock))
        .await;

    match result {
        Err(AgentDriverError::ConversationResumeStateMissing { harness, .. }) => {
            assert_eq!(harness, "codex");
        }
        other => panic!("expected ConversationResumeStateMissing, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_resume_payload_maps_other_errors_to_load_failed() {
    let mut mock = MockHarnessSupportClient::new();
    mock.expect_fetch_transcript()
        .returning(|| Err(anyhow::anyhow!("connection reset")));
    let conversation_id = AIConversationId::new();

    let result = CodexHarness
        .fetch_resume_payload(&conversation_id, Arc::new(mock))
        .await;

    assert!(
        matches!(result, Err(AgentDriverError::ConversationLoadFailed(_))),
        "expected ConversationLoadFailed, got {result:?}"
    );
}

#[test]
#[serial_test::serial]
fn resolve_openai_base_url_from_secret_returns_base_url_when_typed_secret_active() {
    // When the typed OpenAI secret is the active API key source, the base URL
    // should be extracted from the structured secret.
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) };

    let secrets = HashMap::from([(
        "openai-key".to_string(),
        ManagedSecretValue::openai_api_key(
            "sk-test",
            Some("https://us.api.openai.com/v1".to_string()),
        ),
    )]);
    let resolved_env =
        HashMap::from([(OsString::from("OPENAI_API_KEY"), OsString::from("sk-test"))]);

    let result = resolve_openai_base_url_from_secret(&secrets, &resolved_env);

    if let Some(v) = prev {
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) };
    }
    assert_eq!(result.as_deref(), Some("https://us.api.openai.com/v1"));
}

#[test]
#[serial_test::serial]
fn resolve_openai_base_url_from_secret_returns_none_when_worker_env_wins() {
    // When a worker-injected OPENAI_API_KEY already exists in process env,
    // the typed-secret base_url should NOT be applied.
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::set_var(OPENAI_API_KEY_ENV, "sk-worker-key") };

    let secrets = HashMap::from([(
        "openai-key".to_string(),
        ManagedSecretValue::openai_api_key(
            "sk-secret",
            Some("https://us.api.openai.com/v1".to_string()),
        ),
    )]);
    let resolved_env = HashMap::new();

    let result = resolve_openai_base_url_from_secret(&secrets, &resolved_env);

    match prev {
        // TODO: Audit that the environment access only happens in single-threaded code.
        Some(v) => unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) },
        // TODO: Audit that the environment access only happens in single-threaded code.
        None => unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) },
    }
    assert_eq!(result, None);
}

#[test]
#[serial_test::serial]
fn resolve_openai_base_url_from_secret_returns_none_when_no_base_url() {
    // When the typed OpenAI secret has no base_url, None is returned.
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) };

    let secrets = HashMap::from([(
        "openai-key".to_string(),
        ManagedSecretValue::openai_api_key("sk-test", None),
    )]);
    let resolved_env =
        HashMap::from([(OsString::from("OPENAI_API_KEY"), OsString::from("sk-test"))]);

    let result = resolve_openai_base_url_from_secret(&secrets, &resolved_env);

    if let Some(v) = prev {
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) };
    }
    assert_eq!(result, None);
}

#[test]
#[serial_test::serial]
fn resolve_openai_base_url_from_secret_returns_none_when_api_key_not_in_resolved() {
    // When OPENAI_API_KEY is not in the resolved env vars (e.g. the secret was
    // skipped due to collision), the base URL should not be applied.
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    // TODO: Audit that the environment access only happens in single-threaded code.
    unsafe { std::env::remove_var(OPENAI_API_KEY_ENV) };

    let secrets = HashMap::from([(
        "openai-key".to_string(),
        ManagedSecretValue::openai_api_key(
            "sk-test",
            Some("https://us.api.openai.com/v1".to_string()),
        ),
    )]);
    let resolved_env = HashMap::new();

    let result = resolve_openai_base_url_from_secret(&secrets, &resolved_env);

    if let Some(v) = prev {
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(OPENAI_API_KEY_ENV, v) };
    }
    assert_eq!(result, None);
}

#[tokio::test]
async fn fetch_resume_payload_returns_codex_variant_on_success() {
    let uuid = Uuid::new_v4();
    let envelope = CodexTranscriptEnvelope {
        cwd: "/cloud/work".into(),
        session_id: uuid,
        codex_version: Some("0.55.0".to_string()),
        session_start_timestamp: None,
        entries: vec![serde_json::json!({"type": "event_msg"})],
    };
    let bytes = serde_json::to_vec(&envelope).unwrap();

    let mut mock = MockHarnessSupportClient::new();
    mock.expect_fetch_transcript()
        .returning(move || Ok(bytes::Bytes::from(bytes.clone())));
    let conversation_id = AIConversationId::new();

    let payload = CodexHarness
        .fetch_resume_payload(&conversation_id, Arc::new(mock))
        .await
        .unwrap()
        .unwrap();

    match payload {
        ResumePayload::Codex(info) => {
            assert_eq!(info.session_id, uuid);
            assert_eq!(info.conversation_id, conversation_id);
            assert_eq!(info.envelope.codex_version.as_deref(), Some("0.55.0"));
        }
        other => panic!("expected ResumePayload::Codex, got {other:?}"),
    }
}
