use std::ffi::OsString;
use std::fs;
use std::sync::Arc;

use tempfile::TempDir;
use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;

use super::{
    build_local_claude_child_command, build_local_codex_child_command,
    build_local_opencode_child_command, local_child_task_config, local_claude_child_prompt,
    normalize_local_child_harness, prepare_local_harness_child_launch,
    validate_local_harness_shell,
};
use crate::ai::agent_sdk::driver::OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV;
use crate::ai::ambient_agents::task::{HarnessConfig, normalize_orchestrator_agent_name};
use crate::ai::local_harness_setup::LOCAL_CODEX_HARNESS_DISABLED_MESSAGE;
use crate::server::server_api::ai::MockAIClient;
use crate::terminal::shell::ShellType;

struct EnvVarGuard {
    key: &'static str,
    original: Option<OsString>,
}
#[test]
fn local_claude_child_prompt_includes_oz_cli_messaging_instructions() {
    let prompt = local_claude_child_prompt("List files");

    assert!(prompt.contains("OZ_CLI"));
    assert!(prompt.contains("OZ_RUN_ID"));
    assert!(prompt.contains("OZ_PARENT_RUN_ID"));
    assert!(prompt.contains("run message send --sender-run-id"));
    assert!(prompt.contains("All four send arguments are required"));
    assert!(prompt.contains("Do not pass \"$OZ_PARENT_RUN_ID\" as a positional argument to send"));
    assert!(prompt.contains("run message list \"$OZ_RUN_ID\" --limit 25"));
    assert!(prompt.contains("do not rely on --unread"));
    assert!(!prompt.contains("--unread --limit"));
    assert!(prompt.contains("Do not use Claude Code Agent or SendMessage tools"));
    assert!(prompt.ends_with("Task:\nList files"));
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl Into<OsString>) -> Self {
        let original = std::env::var_os(key);
        // TODO: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(key, value.into()) };
        Self { key, original }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(original) = &self.original {
            // TODO: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var(self.key, original) };
        } else {
            // TODO: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::remove_var(self.key) };
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
            local_child_task_config(harness, None),
            Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
                harness: Some(HarnessConfig::from_harness_type(harness)),
                ..Default::default()
            }),
        );
    }
}

#[test]
fn local_child_task_config_stamps_orchestrator_name() {
    for harness in [Harness::Claude, Harness::OpenCode, Harness::Codex] {
        assert_eq!(
            local_child_task_config(harness, Some("frontend-tests".to_string())),
            Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
                name: Some("frontend-tests".to_string()),
                harness: Some(HarnessConfig::from_harness_type(harness)),
                ..Default::default()
            }),
        );
    }
}

#[test]
fn local_child_task_config_trims_whitespace_only_name() {
    assert_eq!(
        local_child_task_config(Harness::Claude, Some("  frontend-tests  ".to_string())),
        Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
            name: Some("frontend-tests".to_string()),
            harness: Some(HarnessConfig::from_harness_type(Harness::Claude)),
            ..Default::default()
        }),
    );
    assert_eq!(
        local_child_task_config(Harness::Claude, Some("   ".to_string())),
        Some(crate::ai::ambient_agents::task::AgentConfigSnapshot {
            name: None,
            harness: Some(HarnessConfig::from_harness_type(Harness::Claude)),
            ..Default::default()
        }),
    );
}

#[test]
fn local_child_task_config_returns_none_for_oz_and_unknown() {
    assert!(local_child_task_config(Harness::Oz, Some("name".to_string())).is_none());
    assert!(local_child_task_config(Harness::Unknown, Some("name".to_string())).is_none());
}

#[test]
fn normalize_orchestrator_agent_name_trims_and_drops_empty() {
    assert_eq!(
        normalize_orchestrator_agent_name("frontend-tests"),
        Some("frontend-tests".to_string())
    );
    assert_eq!(
        normalize_orchestrator_agent_name("  frontend-tests  "),
        Some("frontend-tests".to_string())
    );
    assert_eq!(normalize_orchestrator_agent_name(""), None);
    assert_eq!(normalize_orchestrator_agent_name("   "), None);
    assert_eq!(normalize_orchestrator_agent_name("\t\n  "), None);
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_codex_child_launch_rejects_without_rewriting_global_codex_state() {
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "codex");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _path = EnvVarGuard::set("PATH", fake_bin_dir.path().as_os_str().to_os_string());

    let mut ai_client = MockAIClient::new();
    ai_client.expect_create_agent_task().times(0);

    let result = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "codex".to_string(),
        None,
        Some("parent-run".to_string()),
        None,
        Some(ShellType::Zsh),
        Some(working_dir),
        Arc::new(ai_client),
    )
    .await;

    match result {
        Ok(_) => panic!("disabled local codex should be rejected"),
        Err(err) => assert_eq!(err, LOCAL_CODEX_HARNESS_DISABLED_MESSAGE),
    }
    assert!(!fake_home.path().join(".codex").exists());
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_codex_child_launch_succeeds_when_testing_flag_is_enabled() {
    let _local_codex = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
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
        Some("ignored-model".to_string()),
        Some("parent-run".to_string()),
        None,
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
    assert!(
        !prepared
            .env_vars
            .contains_key(&OsString::from("ANTHROPIC_MODEL"))
    );
    assert_eq!(prepared.run_id, "550e8400-e29b-41d4-a716-446655440000");
    assert!(!fake_home.path().join(".codex").exists());
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_claude_child_merges_anthropic_model_env_var() {
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "claude");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _claude_home = EnvVarGuard::set(
        "CLAUDE_HOME",
        fake_home.path().join(".claude").as_os_str().to_os_string(),
    );
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
        None,
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
    assert!(
        !prepared
            .env_vars
            .contains_key(&OsString::from(OZ_MESSAGE_LISTENER_MANAGED_EXTERNALLY_ENV))
    );
    assert!(
        !prepared
            .env_vars
            .contains_key(&OsString::from("OZ_PARENT_LISTENER_MANAGED_EXTERNALLY"))
    );
    assert!(
        prepared
            .command
            .contains("run message send --sender-run-id")
    );
    assert!(prepared.command.contains("OZ_PARENT_RUN_ID"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_local_claude_child_no_anthropic_model_when_empty() {
    let fake_home = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let working_dir = fake_home.path().join("workspace");
    fs::create_dir_all(&working_dir).unwrap();
    write_fake_cli(fake_bin_dir.path(), "claude");

    let _home = EnvVarGuard::set("HOME", fake_home.path().as_os_str().to_os_string());
    let _claude_home = EnvVarGuard::set(
        "CLAUDE_HOME",
        fake_home.path().join(".claude").as_os_str().to_os_string(),
    );
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
        None,
        Some(ShellType::Zsh),
        Some(working_dir),
        Arc::new(ai_client),
    )
    .await
    .unwrap();

    assert!(
        !prepared
            .env_vars
            .contains_key(&OsString::from("ANTHROPIC_MODEL"))
    );
}

#[tokio::test]
async fn prepare_local_harness_child_launch_rejects_disabled_codex_before_shell_validation() {
    let ai_client = Arc::new(MockAIClient::new());
    let result = prepare_local_harness_child_launch(
        "hello world".to_string(),
        "codex".to_string(),
        None,
        Some("parent-run".to_string()),
        None,
        None,
        None,
        ai_client,
    )
    .await;

    match result {
        Ok(_) => panic!("disabled local codex should be rejected"),
        Err(err) => assert_eq!(err, LOCAL_CODEX_HARNESS_DISABLED_MESSAGE),
    }
}
