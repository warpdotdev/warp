use anyhow::anyhow;
use warp_graphql::ai::{AgentTaskState, PlatformErrorCode};

use super::*;
use crate::ai::agent_sdk::driver::error_classification::classify_driver_error;

fn failed_save() -> Result<(), AgentDriverError> {
    Err(final_save_error(
        "claude",
        anyhow!("native transcript is missing").context("Harness transcript save failed"),
    ))
}

#[test]
fn successful_command_and_save_complete_successfully() {
    assert!(finish_harness_run("claude".into(), Ok(0.into()), None, None, Ok(())).is_ok());
}

#[test]
fn successful_command_cannot_hide_failed_save() {
    let error =
        finish_harness_run("claude".into(), Ok(0.into()), None, None, failed_save()).unwrap_err();
    let AgentDriverError::HarnessConversationSaveFailed { harness, detail } = &error else {
        panic!("expected persistence failure, got {error:?}");
    };
    assert_eq!(harness, "claude");
    assert_eq!(
        detail,
        "Harness transcript save failed: native transcript is missing"
    );
    let (state, update) = classify_driver_error(&error);
    assert_eq!(state, AgentTaskState::Failed);
    assert_eq!(update.error_code, Some(PlatformErrorCode::InternalError));
    assert!(update.message.contains("Follow-ups may be unavailable"));
    let platform_error = update.platform_error.unwrap();
    assert!(!platform_error.retryable);
    assert_eq!(platform_error.detail.as_deref(), Some(detail.as_str()));
}

#[test]
fn failed_command_keeps_its_output_when_save_also_fails() {
    let error = finish_harness_run(
        "claude".into(),
        Ok(7.into()),
        Some("command failure".into()),
        None,
        failed_save(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AgentDriverError::HarnessCommandFailed { exit_code: 7, output }
            if output.as_deref() == Some("command failure")
    ));
}

#[test]
fn command_transport_failure_takes_precedence_over_save_failure() {
    let error = finish_harness_run(
        "claude".into(),
        Err(AgentDriverError::InvalidRuntimeState),
        None,
        None,
        failed_save(),
    )
    .unwrap_err();
    assert!(matches!(error, AgentDriverError::InvalidRuntimeState));
}

#[test]
fn detected_runtime_failure_takes_precedence_over_command_and_save_failures() {
    let error = finish_harness_run(
        "claude".into(),
        Err(AgentDriverError::InvalidRuntimeState),
        None,
        Some(DetectedHarnessError {
            pattern: "invalid API key".into(),
            excerpt: "authentication failed".into(),
        }),
        failed_save(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        AgentDriverError::HarnessRuntimeFailureDetected { pattern, .. }
            if pattern == "invalid API key"
    ));
}

#[test]
fn save_error_retains_context_without_signed_urls_or_credentials() {
    let error = final_save_error(
        "claude",
        anyhow!(
            "HTTP 403 uploading https://user:password@example.com/object?X-Goog-Signature=signed-secret \
             with credential AKIAIOSFODNN7EXAMPLE"
        )
        .context("Harness transcript upload failed"),
    );
    let debug = format!("{error:?}");
    assert!(debug.contains("Harness transcript upload failed"));
    assert!(debug.contains("HTTP 403"));
    for sensitive in [
        "password",
        "example.com",
        "signed-secret",
        "AKIAIOSFODNN7EXAMPLE",
    ] {
        assert!(!debug.contains(sensitive), "unredacted diagnostic");
    }
}

#[test]
fn save_error_diagnostic_is_bounded_and_preserves_unicode() {
    let error = final_save_error("claude", anyhow!("START{}END", "🙂".repeat(4096)));
    let AgentDriverError::HarnessConversationSaveFailed { detail, .. } = error else {
        unreachable!();
    };
    assert!(detail.len() <= 4096);
    assert!(detail.starts_with("START"));
    assert!(detail.ends_with("END"));
}
