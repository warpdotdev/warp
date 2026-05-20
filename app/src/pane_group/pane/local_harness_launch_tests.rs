use std::{ffi::OsString, fs, sync::Arc};

use tempfile::TempDir;
use warp_cli::agent::Harness;

use super::{
    build_local_claude_child_command, build_local_codex_child_command,
    build_local_opencode_child_command, local_child_task_config, normalize_local_child_harness,
    prepare_local_harness_child_launch, validate_local_harness_shell,
};
use crate::ai::ambient_agents::task::HarnessConfig;
use crate::server::server_api::ai::MockAIClient;
use crate::terminal::shell::ShellType;
use warp_core::features::FeatureFlag;

struct EnvVarGuard {
    key: &'static str,
    original: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl Into<OsString>) -> Self {
        let original = std::env::var_os(key);
        std::env::set_var(key, value.into());
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(original) = &self.original {
            std::env::set_var(self.key, original);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn write_fake_cli(bin_dir: &std::path::Path, name: &str) {
    let executable_name = if cfg!(windows) {
        format!("{name}.cmd")
    } else {
        name.to_string()
    };
    let executable_path = bin_dir.join(executable_name);
    let script = if cfg!(windows) {
        "@echo off\r\n"
    } else {
        "#!/bin/sh\n"
    };

    fs::write(&executable_path, script).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&executable_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable_path, permissions).unwrap();
    }
}

#[test]
fn normalize_local_child_harness_accepts_supported_aliases() {
    assert_eq!(
        normalize_local_child_harness("claude"),
        Some(Harness::Claude)
    );
    assert_eq!(
        normalize_local_child_harness("claude-code"),
        Some(Harness::Claude)
    );
    assert_eq!(
        normalize_local_child_harness("claude_code"),
        Some(Harness::Claude)
    );
    assert_eq!(
        normalize_local_child_harness("opencode"),
        Some(Harness::OpenCode)
    );
    assert_eq!(
        normalize_local_child_harness("open-code"),
        Some(Harness::OpenCode)
    );
    assert_eq!(
        normalize_local_child_harness("open_code"),
        Some(Harness::OpenCode)
    );
    assert_eq!(normalize_local_child_harness("codex"), Some(Harness::Codex));
}

#[test]
fn normalize_local_child_harness_rejects_unsupported_values() {
    assert_eq!(normalize_local_child_harness("oz"), None);
    assert_eq!(normalize_local_child_harness("gemini"), None);
    assert_eq!(normalize_local_child_harness(""), None);
}

#[test]
fn validate_local_harness_shell_accepts_supported_shells() {
    assert_eq!(validate_local_harness_shell(Some(ShellType::Bash)), Ok(()));
    assert_eq!(validate_local_harness_shell(Some(ShellType::Zsh)), Ok(()));
    assert_eq!(validate_local_harness_shell(Some(ShellType::Fish)), Ok(()));
}

#[test]
fn validate_local_harness_shell_rejects_unsupported_shells() {
    assert_eq!(
        validate_local_harness_shell(Some(ShellType::PowerShell)),
        Err(
            "Local child harnesses currently require bash, zsh, or fish; PowerShell is not supported."
                .to_string()
        )
    );
    assert_eq!(
        validate_local_harness_shell(None),
        Err(
            "Local child harnesses currently require a detected bash, zsh, or fish session."
                .to_string()
        )
    );
}

#[test]
fn build_local_claude_child_command_quotes_the_prompt() {
    let command = build_local_claude_child_command("hello world");

    assert!(command.starts_with("claude --session-id "));
    assert!(command.ends_with(" --dangerously-skip-permissions 'hello world'"));
}

#[test]
fn build_local_opencode_child_command_quotes_the_prompt() {
    assert_eq!(
        build_local_opencode_child_command("hello world"),
        "opencode --prompt 'hello world'"
    );
}

#[test]
fn build_local_codex_child_command_quotes_the_prompt() {
    assert_eq!(
        build_local_codex_child_command("hello world"),
        "codex --dangerously-bypass-approvals-and-sandbox 'hello world'"
    );
}

#[test]
fn local_child_task_config_records_supported_third_party_harnesses() {
    for harness in [Harness::Claude, Harness::OpenCode, Harness::Codex] {
        assert_eq!(
            local_child_task_config(harness),
            Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
                harness: Some(HarnessConfig::from_harness_type(harness)),
                ..Default::default()
            }),
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_codex_child_launch_does_not_rewrite_global_codex_state() {
    let _local_harnesses = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "codex");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path().as_os_str().to_os_string());

    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_create_agent_task()
        .times(1)
        .returning(|_, _, _, _| Ok("550e8400-e29b-41d4-a716-446655440000".parse().unwrap()));

    let prepared = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "codex".to_string(),
        None,
        Some("parent-run".to_string()),
        Some(ShellType::Zsh),
        Some(working_dir),
        Arc::new(ai_client),
    )
    .await
    .unwrap();

    assert_eq!(
        prepared.command,
        "codex --dangerously-bypass-approvals-and-sandbox 'hello world'"
    );
    assert!(!fake_home.path().join(".codex").exists());
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_claude_child_merges_anthropic_model_env_var() {
    let _local_harnesses = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "claude");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path().as_os_str().to_os_string());

    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_create_agent_task()
        .times(1)
        .returning(|_, _, _, _| Ok("550e8400-e29b-41d4-a716-446655440000".parse().unwrap()));

    let prepared = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "claude".to_string(),
        Some("opus".to_string()),
        Some("parent-run".to_string()),
        Some(ShellType::Zsh),
        Some(working_dir),
        Arc::new(ai_client),
    )
    .await
    .unwrap();

    assert_eq!(
        prepared.env_vars.get(&OsString::from("ANTHROPIC_MODEL")),
        Some(&OsString::from("opus"))
    );
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_claude_child_no_anthropic_model_when_empty() {
    let _local_harnesses = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "claude");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path().as_os_str().to_os_string());

    let mut ai_client = MockAIClient::new();
    ai_client
        .expect_create_agent_task()
        .times(1)
        .returning(|_, _, _, _| Ok("550e8400-e29b-41d4-a716-446655440000".parse().unwrap()));

    let prepared = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "claude".to_string(),
        None,
        Some("parent-run".to_string()),
        Some(ShellType::Zsh),
        Some(working_dir),
        Arc::new(ai_client),
    )
    .await
    .unwrap();

    assert!(!prepared
        .env_vars
        .contains_key(&OsString::from("ANTHROPIC_MODEL")));
}

#[tokio::test]
async fn prepare_local_harness_child_launch_rejects_disabled_claude_before_shell_validation() {
    let ai_client = Arc::new(MockAIClient::new());
    let result = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "claude".to_string(),
        None,
        Some("parent-run".to_string()),
        None,
        None,
        ai_client,
    )
    .await;

    match result {
        Ok(_) => panic!("disabled local claude should be rejected"),
        Err(err) => assert_eq!(
            err,
            "Local Claude Code child agents are temporarily disabled."
        ),
    }
}
