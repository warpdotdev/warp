use std::ffi::{OsStr, OsString};

use warp_cli::agent::Harness;

use super::{auth_check_command_for, harness_model_env_vars, validate_cli_installed};
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::ai::ambient_agents::task::HarnessModelConfig;

fn assert_harness_setup_failed(err: &AgentDriverError) -> (&str, &str) {
    match err {
        AgentDriverError::HarnessSetupFailed { harness, reason } => (harness, reason),
        other => panic!("expected HarnessSetupFailed, got: {other}"),
    }
}

#[cfg(not(windows))]
#[test]
fn validate_cli_installed_succeeds_for_known_binary() {
    assert!(validate_cli_installed("ls", None).is_ok());
}

#[test]
fn validate_cli_installed_fails_for_missing_binary() {
    let err = validate_cli_installed("__nonexistent_cli_abc123__", None).unwrap_err();
    let (harness, reason) = assert_harness_setup_failed(&err);
    assert_eq!(harness, "__nonexistent_cli_abc123__");
    assert!(reason.contains("not found"));
    assert!(!reason.contains("Install it first"));
}

#[test]
fn validate_cli_installed_includes_docs_url_in_error() {
    let url = "https://example.com/install";
    let err = validate_cli_installed("__nonexistent_cli_abc123__", Some(url)).unwrap_err();
    let (_, reason) = assert_harness_setup_failed(&err);
    assert!(reason.contains(url));
    assert!(reason.contains("Install it first"));
}

// --- Runtime error pattern tests ---

#[test]
fn claude_runtime_error_patterns_returns_slice() {
    use super::ThirdPartyHarness;
    use super::claude_code::ClaudeHarness;
    // Patterns are initially empty until validated needles are filled in.
    // The trait method must still be callable.
    let _: &[&str] = ClaudeHarness.runtime_error_patterns();
}

#[test]
fn codex_runtime_error_patterns_returns_slice() {
    use super::ThirdPartyHarness;
    use super::codex::CodexHarness;
    let _: &[&str] = CodexHarness.runtime_error_patterns();
}

#[test]
fn gemini_runtime_error_patterns_is_empty_by_default() {
    use super::ThirdPartyHarness;
    use super::gemini::GeminiHarness;
    assert!(GeminiHarness.runtime_error_patterns().is_empty());
}

#[test]
fn auth_check_command_for_gemini_is_none() {
    assert!(auth_check_command_for(Harness::Gemini).is_none());
}

#[test]
fn auth_check_command_for_oz_is_none() {
    assert!(auth_check_command_for(Harness::Oz).is_none());
}

#[test]
fn auth_check_command_for_unsupported_is_none() {
    // OpenCode is mapped to HarnessKind::Unsupported and therefore has no
    // auth check command of its own.
    assert!(auth_check_command_for(Harness::OpenCode).is_none());
}

#[test]
fn auth_check_command_for_unknown_is_none() {
    // Harness::Unknown causes harness_kind to return Err; the helper still
    // returns None instead of panicking.
    assert!(auth_check_command_for(Harness::Unknown).is_none());
}

// --- harness_model_env_vars tests ---

#[test]
fn claude_model_env_vars_sets_model_and_effort_level() {
    let config = HarnessModelConfig {
        model_id: "opus".to_owned(),
        reasoning_level: Some("xhigh".to_owned()),
    };
    let env_vars = harness_model_env_vars(Harness::Claude, Some(&config));
    assert_eq!(
        env_vars.get(OsStr::new("ANTHROPIC_MODEL")),
        Some(&OsString::from("opus"))
    );
    assert_eq!(
        env_vars.get(OsStr::new("CLAUDE_CODE_EFFORT_LEVEL")),
        Some(&OsString::from("xhigh"))
    );
}

#[test]
fn claude_model_env_vars_omits_effort_level_when_unset() {
    // No reasoning level chosen: CLAUDE_CODE_EFFORT_LEVEL must be left unset so
    // Claude Code falls back to its own per-model default effort.
    let config = HarnessModelConfig {
        model_id: "haiku".to_owned(),
        reasoning_level: None,
    };
    let env_vars = harness_model_env_vars(Harness::Claude, Some(&config));
    assert_eq!(
        env_vars.get(OsStr::new("ANTHROPIC_MODEL")),
        Some(&OsString::from("haiku"))
    );
    assert!(!env_vars.contains_key(OsStr::new("CLAUDE_CODE_EFFORT_LEVEL")));
}

#[test]
fn claude_model_env_vars_omits_effort_level_when_empty() {
    let config = HarnessModelConfig {
        model_id: "opus".to_owned(),
        reasoning_level: Some(String::new()),
    };
    let env_vars = harness_model_env_vars(Harness::Claude, Some(&config));
    assert!(!env_vars.contains_key(OsStr::new("CLAUDE_CODE_EFFORT_LEVEL")));
}

#[test]
fn non_claude_model_env_vars_never_set_effort_level() {
    // Codex has its own reasoning-effort mechanism (config.toml); the Claude-
    // specific env var must not leak into other harnesses.
    let config = HarnessModelConfig {
        model_id: "gpt-5.5".to_owned(),
        reasoning_level: Some("high".to_owned()),
    };
    let env_vars = harness_model_env_vars(Harness::Codex, Some(&config));
    assert!(!env_vars.contains_key(OsStr::new("CLAUDE_CODE_EFFORT_LEVEL")));
    assert!(!env_vars.contains_key(OsStr::new("ANTHROPIC_MODEL")));
}

#[test]
fn model_env_vars_empty_when_no_model_config() {
    assert!(harness_model_env_vars(Harness::Claude, None).is_empty());
}
