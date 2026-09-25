use warp_cli::agent::Harness;

use super::{
    HARNESS_FAILURE_OUTPUT_MAX_BYTES, HARNESS_FAILURE_OUTPUT_TRUNCATION_MARKER,
    auth_check_command_for, prepare_harness_failure_output, truncate_harness_failure_output,
    validate_cli_installed,
};
use crate::ai::agent_sdk::driver::AgentDriverError;

fn assert_harness_setup_failed(err: &AgentDriverError) -> (&str, &str) {
    match err {
        AgentDriverError::HarnessSetupFailed { harness, reason } => (harness, reason),
        other => panic!("expected HarnessSetupFailed, got: {other}"),
    }
}

#[test]
fn short_harness_failure_output_is_preserved() {
    let output = "Harness startup\nRequest failed: invalid credentials";

    assert_eq!(truncate_harness_failure_output(output), output);
}

#[test]
fn harness_failure_output_at_byte_limit_is_preserved() {
    let output = format!(
        "START{}END",
        "x".repeat(HARNESS_FAILURE_OUTPUT_MAX_BYTES - "START".len() - "END".len())
    );

    assert_eq!(output.len(), 4_096);
    assert_eq!(truncate_harness_failure_output(&output), output);
}

#[test]
fn harness_failure_output_one_byte_over_limit_retains_its_start_and_end() {
    let output = format!(
        "START{}END",
        "x".repeat(HARNESS_FAILURE_OUTPUT_MAX_BYTES + 1 - "START".len() - "END".len())
    );

    let truncated = truncate_harness_failure_output(&output);

    assert_eq!(output.len(), 4_097);
    assert!(truncated.len() <= 4_096);
    assert!(truncated.starts_with("START"));
    assert!(truncated.ends_with("END"));
    assert!(truncated.contains(HARNESS_FAILURE_OUTPUT_TRUNCATION_MARKER));
}

#[test]
fn harness_failure_output_is_redacted_before_leaving_the_client() {
    let secret = "AKIAIOSFODNN7EXAMPLE";
    let output = format!("Harness failed with credential {secret}");

    let prepared = prepare_harness_failure_output(&output);

    assert!(!prepared.contains(secret));
    assert_eq!(
        prepared,
        format!(
            "Harness failed with credential {}",
            "*".repeat(secret.len())
        )
    );
}

#[test]
fn harness_failure_output_truncation_preserves_unicode_boundaries() {
    let output = format!(
        "START-世{}界-END",
        "🙂".repeat(HARNESS_FAILURE_OUTPUT_MAX_BYTES)
    );

    let truncated = truncate_harness_failure_output(&output);

    assert!(truncated.len() <= HARNESS_FAILURE_OUTPUT_MAX_BYTES);
    assert!(truncated.starts_with("START-世"));
    assert!(truncated.ends_with("界-END"));
    assert!(truncated.contains(HARNESS_FAILURE_OUTPUT_TRUNCATION_MARKER));
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
