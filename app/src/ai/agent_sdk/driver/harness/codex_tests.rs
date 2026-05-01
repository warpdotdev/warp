use std::collections::HashMap;
use std::fs;

use serde_json::Value;
use tempfile::TempDir;

use super::*;

#[test]
fn prepare_codex_auth_writes_fresh_file_with_api_key_mode() {
    let tmp = TempDir::new().unwrap();
    let auth_path = tmp.path().join(".codex/auth.json");

    prepare_codex_auth(&auth_path, "sk-test-key").unwrap();

    let auth: Value = serde_json::from_slice(&fs::read(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["OPENAI_API_KEY"], "sk-test-key");
    assert_eq!(auth["auth_mode"], "apikey");
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
fn resolve_openai_api_key_returns_value_from_raw_value_secret() {
    let secrets = HashMap::from([(
        "OPENAI_API_KEY".to_string(),
        ManagedSecretValue::raw_value("sk-from-secret"),
    )]);
    assert_eq!(
        resolve_openai_api_key(&secrets).as_deref(),
        Some("sk-from-secret")
    );
}

#[test]
#[serial_test::serial]
fn resolve_openai_api_key_falls_back_to_env_var() {
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    std::env::set_var(OPENAI_API_KEY_ENV, "sk-from-env");

    let result = resolve_openai_api_key(&HashMap::new());

    match prev {
        Some(v) => std::env::set_var(OPENAI_API_KEY_ENV, v),
        None => std::env::remove_var(OPENAI_API_KEY_ENV),
    }
    assert_eq!(result.as_deref(), Some("sk-from-env"));
}

#[test]
#[serial_test::serial]
fn resolve_openai_api_key_returns_none_when_secrets_and_env_empty() {
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    std::env::remove_var(OPENAI_API_KEY_ENV);

    let result = resolve_openai_api_key(&HashMap::new());

    if let Some(v) = prev {
        std::env::set_var(OPENAI_API_KEY_ENV, v);
    }
    assert_eq!(result, None);
}

#[test]
#[serial_test::serial]
fn resolve_openai_api_key_prefers_env_over_secret() {
    // Mirrors `AgentDriver::new`'s precedence: an existing `OPENAI_API_KEY` env var
    // wins over a managed secret so `auth.json` matches the launched process's env.
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    std::env::set_var(OPENAI_API_KEY_ENV, "sk-from-env");
    let secrets = HashMap::from([(
        "OPENAI_API_KEY".to_string(),
        ManagedSecretValue::raw_value("sk-from-secret"),
    )]);

    let result = resolve_openai_api_key(&secrets);

    match prev {
        Some(v) => std::env::set_var(OPENAI_API_KEY_ENV, v),
        None => std::env::remove_var(OPENAI_API_KEY_ENV),
    }
    assert_eq!(result.as_deref(), Some("sk-from-env"));
}

#[test]
#[serial_test::serial]
fn resolve_openai_api_key_uses_secret_when_env_empty() {
    let prev = std::env::var(OPENAI_API_KEY_ENV).ok();
    std::env::set_var(OPENAI_API_KEY_ENV, "   ");
    let secrets = HashMap::from([(
        "OPENAI_API_KEY".to_string(),
        ManagedSecretValue::raw_value("sk-from-secret"),
    )]);

    let result = resolve_openai_api_key(&secrets);

    match prev {
        Some(v) => std::env::set_var(OPENAI_API_KEY_ENV, v),
        None => std::env::remove_var(OPENAI_API_KEY_ENV),
    }
    assert_eq!(result.as_deref(), Some("sk-from-secret"));
}

fn read_codex_config(path: &std::path::Path) -> toml::Table {
    let content = fs::read_to_string(path).unwrap();
    toml::from_str(&content).unwrap()
}

#[test]
fn prepare_codex_config_toml_writes_fresh_config() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join(".codex/config.toml");
    let working_dir = tmp.path().join("workspace/proj");
    fs::create_dir_all(&working_dir).unwrap();

    prepare_codex_config_toml(&config_path, &working_dir).unwrap();

    let canonical = working_dir.canonicalize().unwrap();
    let key = canonical.to_string_lossy().into_owned();
    let cfg = read_codex_config(&config_path);
    assert_eq!(
        cfg["projects"][&key]["trust_level"].as_str(),
        Some("trusted")
    );
    assert_eq!(cfg["openai_base_url"].as_str(), Some(CODEX_OPENAI_BASE_URL));
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

    prepare_codex_config_toml(&config_path, &working_dir).unwrap();

    let canonical = working_dir.canonicalize().unwrap();
    let key = canonical.to_string_lossy().into_owned();
    let cfg = read_codex_config(&config_path);
    assert_eq!(cfg["model"].as_str(), Some("gpt-5"));
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

    prepare_codex_config_toml(&config_path, &working_dir).unwrap();
    let after_first = fs::read_to_string(&config_path).unwrap();
    prepare_codex_config_toml(&config_path, &working_dir).unwrap();
    let after_second = fs::read_to_string(&config_path).unwrap();

    assert_eq!(after_first, after_second);
    let canonical = working_dir.canonicalize().unwrap();
    let key = canonical.to_string_lossy().into_owned();
    let cfg: toml::Table = toml::from_str(&after_second).unwrap();
    let projects = cfg["projects"].as_table().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[&key]["trust_level"].as_str(), Some("trusted"));
    assert_eq!(cfg["openai_base_url"].as_str(), Some(CODEX_OPENAI_BASE_URL));
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

    prepare_codex_config_toml(&config_path, &working_dir).unwrap();

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

    prepare_codex_config_toml(&config_path, &working_dir).unwrap();

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

    prepare_codex_config_toml(&config_path, &working_dir).unwrap();

    let cfg = read_codex_config(&config_path);
    assert_eq!(cfg["openai_base_url"].as_str(), Some(CODEX_OPENAI_BASE_URL));
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
