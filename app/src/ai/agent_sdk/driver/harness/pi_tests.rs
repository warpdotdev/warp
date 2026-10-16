use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use warp_cli::agent::{Harness, HarnessTransport};
use warp_managed_secrets::ManagedSecretValue;

use super::super::harness_env_vars;
use super::{
    PI_CODING_AGENT_DIR_ENV, pi_mcp_config, prepare_pi_environment_config, prepare_pi_settings,
    prepare_pi_system_prompt, publish_skills_for_pi,
};
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::mcp::JSONMCPServer;

struct EnvVarGuard {
    key: &'static str,
    original: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: Option<&str>) -> Self {
        let original = std::env::var_os(key);
        unsafe {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.original {
                Some(original) => std::env::set_var(self.key, original),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[test]
#[serial_test::serial]
fn worker_env_overrides_legacy_credentials_without_their_endpoint() {
    let _openai = EnvVarGuard::set("OPENAI_API_KEY", Some("worker-openai"));
    let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", Some("worker-anthropic"));
    let (mut env, guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let dir = guard.unwrap();
    env.extend([
        (
            OsString::from("OPENAI_API_KEY"),
            OsString::from("managed-openai"),
        ),
        (
            OsString::from("ANTHROPIC_API_KEY"),
            OsString::from("managed-anthropic"),
        ),
    ]);
    let secrets = HashMap::from([
        (
            "OPENAI".to_owned(),
            ManagedSecretValue::openai_api_key(
                "managed-openai",
                Some("https://managed.test/v1".to_owned()),
            ),
        ),
        (
            "ANTHROPIC".to_owned(),
            ManagedSecretValue::anthropic_api_key("managed-anthropic"),
        ),
    ]);

    prepare_pi_environment_config(dir.path(), None, &env, &HashMap::new(), &secrets, None).unwrap();

    let auth: Value =
        serde_json::from_slice(&fs::read(dir.path().join("auth.json")).unwrap()).unwrap();
    assert_eq!(auth, json!({}));
    assert!(!dir.path().join("models.json").exists());
}

#[test]
fn provider_credentials_are_private_literal_values_with_endpoint_overrides() {
    let (env, guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let dir = guard.unwrap();
    let secrets = HashMap::from([
        (
            "GOOGLE".to_owned(),
            ManagedSecretValue::llm_provider_api_key("google", "!literal$TOKEN", None),
        ),
        (
            "OPENAI".to_owned(),
            ManagedSecretValue::llm_provider_api_key(
                "openai",
                "${TOKEN}",
                Some("https://example.test/v1".to_owned()),
            ),
        ),
    ]);
    let model = HarnessModelConfig {
        model_id: "google/gemini-2.5-pro".to_owned(),
        reasoning_level: None,
    };

    prepare_pi_environment_config(
        dir.path(),
        None,
        &env,
        &HashMap::new(),
        &secrets,
        Some(&model),
    )
    .unwrap();

    let auth: Value =
        serde_json::from_slice(&fs::read(dir.path().join("auth.json")).unwrap()).unwrap();
    assert_eq!(
        auth,
        json!({
            "google": {"type": "api_key", "key": "$!literal$$TOKEN"},
            "openai": {"type": "api_key", "key": "$${TOKEN}"}
        })
    );
    let models: Value =
        serde_json::from_slice(&fs::read(dir.path().join("models.json")).unwrap()).unwrap();
    assert_eq!(
        models,
        json!({"providers": {"openai": {"baseUrl": "https://example.test/v1"}}})
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(dir.path().join("auth.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(dir.path().join("models.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn default_model_keeps_pi_model_selection_implicit() {
    let (env, guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let dir = guard.unwrap();
    let model = HarnessModelConfig {
        model_id: "default".to_owned(),
        reasoning_level: None,
    };

    prepare_pi_environment_config(
        dir.path(),
        None,
        &env,
        &HashMap::new(),
        &HashMap::new(),
        Some(&model),
    )
    .unwrap();

    let settings: Value =
        serde_json::from_slice(&fs::read(dir.path().join("settings.json")).unwrap()).unwrap();
    assert!(settings.get("defaultProvider").is_none());
    assert!(settings.get("defaultModel").is_none());
}

#[test]
fn generic_provider_binding_wins_over_legacy_auth_for_the_same_provider() {
    let (mut env, guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let dir = guard.unwrap();
    env.insert(OsString::from("OPENAI_API_KEY"), OsString::from("legacy"));
    let secrets = HashMap::from([
        (
            "LEGACY".to_owned(),
            ManagedSecretValue::openai_api_key("legacy", Some("https://legacy.test".to_owned())),
        ),
        (
            "BOUND".to_owned(),
            ManagedSecretValue::llm_provider_api_key("openai", "bound", None),
        ),
    ]);

    prepare_pi_environment_config(dir.path(), None, &env, &HashMap::new(), &secrets, None).unwrap();

    let auth: Value =
        serde_json::from_slice(&fs::read(dir.path().join("auth.json")).unwrap()).unwrap();
    assert_eq!(auth, json!({"openai": {"type": "api_key", "key": "bound"}}));
    assert!(!dir.path().join("models.json").exists());
}

#[test]
#[serial_test::serial]
fn legacy_auth_uses_only_the_resolved_key_and_its_endpoint() {
    let _openai = EnvVarGuard::set("OPENAI_API_KEY", None);
    let _anthropic = EnvVarGuard::set("ANTHROPIC_API_KEY", None);
    let (mut env, guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let dir = guard.unwrap();
    env.extend([
        (OsString::from("OPENAI_API_KEY"), OsString::from("active")),
        (
            OsString::from("ANTHROPIC_API_KEY"),
            OsString::from("anthropic"),
        ),
    ]);
    let secrets = HashMap::from([
        (
            "ACTIVE".to_owned(),
            ManagedSecretValue::openai_api_key("active", Some("https://active.test/v1".to_owned())),
        ),
        (
            "OTHER".to_owned(),
            ManagedSecretValue::openai_api_key(
                "ignored",
                Some("https://ignored.test/v1".to_owned()),
            ),
        ),
        (
            "ANTHROPIC".to_owned(),
            ManagedSecretValue::anthropic_api_key("anthropic"),
        ),
    ]);

    prepare_pi_environment_config(dir.path(), None, &env, &HashMap::new(), &secrets, None).unwrap();

    let auth: Value =
        serde_json::from_slice(&fs::read(dir.path().join("auth.json")).unwrap()).unwrap();
    assert_eq!(
        auth,
        json!({
            "openai": {"type": "api_key", "key": "active"},
            "anthropic": {"type": "api_key", "key": "anthropic"}
        })
    );
    let models: Value =
        serde_json::from_slice(&fs::read(dir.path().join("models.json")).unwrap()).unwrap();
    assert_eq!(
        models,
        json!({"providers": {"openai": {"baseUrl": "https://active.test/v1"}}})
    );
}

#[test]
fn conflicting_provider_credentials_fail_without_exposing_keys() {
    let (env, guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let dir = guard.unwrap();
    let secrets = HashMap::from([
        (
            "FIRST".to_owned(),
            ManagedSecretValue::llm_provider_api_key("google", "first-key", None),
        ),
        (
            "SECOND".to_owned(),
            ManagedSecretValue::llm_provider_api_key("google", "second-key", None),
        ),
    ]);

    let error =
        prepare_pi_environment_config(dir.path(), None, &env, &HashMap::new(), &secrets, None)
            .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Conflicting Pi credentials for the same provider"
    );
    assert!(!dir.path().join("auth.json").exists());
}

#[test]
fn identical_provider_credentials_coalesce() {
    let (env, guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let dir = guard.unwrap();
    let secrets = HashMap::from([
        (
            "FIRST".to_owned(),
            ManagedSecretValue::llm_provider_api_key("google", "same", None),
        ),
        (
            "SECOND".to_owned(),
            ManagedSecretValue::llm_provider_api_key("google", "same", None),
        ),
    ]);

    prepare_pi_environment_config(dir.path(), None, &env, &HashMap::new(), &secrets, None).unwrap();

    let auth: Value =
        serde_json::from_slice(&fs::read(dir.path().join("auth.json")).unwrap()).unwrap();
    assert_eq!(auth, json!({"google": {"type": "api_key", "key": "same"}}));
}

#[test]
fn invalid_provider_credentials_never_write_auth() {
    for (provider, key) in [("../openai", "key"), ("__proto__", "key"), ("google", "")] {
        let (env, guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
        let dir = guard.unwrap();
        let secrets = HashMap::from([(
            "KEY".to_owned(),
            ManagedSecretValue::llm_provider_api_key(provider, key, None),
        )]);

        let error =
            prepare_pi_environment_config(dir.path(), None, &env, &HashMap::new(), &secrets, None)
                .unwrap_err();

        assert_eq!(error.to_string(), "Invalid LLM provider API key credential");
        assert!(!dir.path().join("auth.json").exists());
    }
}

#[test]
fn environment_allocates_isolated_session_directories() {
    let (first, first_guard) = harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let (second, second_guard) =
        harness_env_vars(Harness::Pi, HarnessTransport::Acp, None).unwrap();
    let first_agent_dir = PathBuf::from(&first[OsStr::new(PI_CODING_AGENT_DIR_ENV)]);
    let second_agent_dir = PathBuf::from(&second[OsStr::new(PI_CODING_AGENT_DIR_ENV)]);
    let first_session_dir = PathBuf::from(&first[OsStr::new("PI_CODING_AGENT_SESSION_DIR")]);
    let second_session_dir = PathBuf::from(&second[OsStr::new("PI_CODING_AGENT_SESSION_DIR")]);

    assert_ne!(first_agent_dir, second_agent_dir);
    assert_eq!(first_session_dir, first_agent_dir.join("sessions"));
    assert_eq!(second_session_dir, second_agent_dir.join("sessions"));
    assert!(first_session_dir.is_dir());
    assert!(second_session_dir.is_dir());

    drop(first_guard);
    assert!(!first_agent_dir.exists());
    assert!(second_agent_dir.exists());
    drop(second_guard);
    assert!(!second_agent_dir.exists());
}

#[test]
fn settings_preserve_existing_values_and_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("settings.json");
    fs::write(
        &settings_path,
        json!({
            "theme": "custom",
            "packages": ["npm:another-package"],
            "extensions": ["custom"]
        })
        .to_string(),
    )
    .unwrap();
    for _ in 0..2 {
        prepare_pi_settings(&settings_path).unwrap();
    }

    let settings: Value = serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
    assert_eq!(settings["theme"], "custom");
    assert_eq!(
        settings["packages"],
        json!(["npm:another-package", "npm:pi-mcp-adapter@5.2.0"])
    );
    assert_eq!(settings["extensions"], json!(["custom", "-builtin:mcp"]));
    assert_eq!(settings["defaultProjectTrust"], "always");
}

#[test]
fn skills_are_published_into_the_agent_skill_root() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let source = workspace.join("factory/skills/factory-skill");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("SKILL.md"), "---\nname: factory-skill\n---\n").unwrap();
    let agent_dir = dir.path().join("agent");
    fs::create_dir_all(&agent_dir).unwrap();

    publish_skills_for_pi(
        &workspace,
        &agent_dir,
        &[PathBuf::from("factory/skills")],
        false,
    )
    .unwrap();

    let link = agent_dir.join("skills/factory-skill");
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(link.canonicalize().unwrap(), source.canonicalize().unwrap());
    assert!(!workspace.join(".pi").exists());
}

#[test]
fn system_prompt_uses_append_slot_without_project_overrides() {
    let dir = tempfile::tempdir().unwrap();
    prepare_pi_system_prompt(dir.path(), &dir.path().join("repo"), "server instructions").unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("APPEND_SYSTEM.md")).unwrap(),
        "server instructions"
    );
    assert!(!dir.path().join("AGENTS.md").exists());
}

#[test]
fn project_append_prompt_routes_server_prompt_to_agent_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let cwd = dir.path().join("repo");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::create_dir_all(cwd.join(".pi")).unwrap();
    fs::write(cwd.join(".pi/APPEND_SYSTEM.md"), "project instructions").unwrap();
    prepare_pi_system_prompt(&agent_dir, &cwd, "server instructions").unwrap();

    assert_eq!(
        fs::read_to_string(agent_dir.join("AGENTS.md")).unwrap(),
        "server instructions"
    );
    assert!(!agent_dir.join("APPEND_SYSTEM.md").exists());
    assert_eq!(
        fs::read_to_string(cwd.join(".pi/APPEND_SYSTEM.md")).unwrap(),
        "project instructions"
    );
}

#[test]
fn mcp_adapter_configuration_preserves_stdio_and_remote_fields() {
    let servers: HashMap<String, JSONMCPServer> = serde_json::from_value(json!({
        "stdio": {"command": "node", "args": ["server.mjs"], "env": {"NAME": "literal"}},
        "remote": {"url": "https://example.test/mcp", "headers": {"X-Test": "literal"}}
    }))
    .unwrap();
    assert_eq!(
        pi_mcp_config(&servers),
        json!({"mcpServers": {
            "stdio": {"command": "node", "args": ["server.mjs"], "env": {"NAME": "literal"}},
            "remote": {"url": "https://example.test/mcp", "headers": {"X-Test": "literal"}}
        }})
    );
}
