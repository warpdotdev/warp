use serde_json::json;
use warp_core::telemetry::TelemetryEventDesc;

use super::{
    CloudAgentShellExitDetection, CloudAgentShellRecoveryFailureClass,
    CloudAgentShellRecoveryOutcome, TelemetryEvent,
};

#[derive(Debug)]
enum TelemetryEventPropertyError {
    // The variant data is never directly read, but it's used for error formatting if the test
    // below fails.
    EmptyName(#[expect(dead_code)] Box<dyn TelemetryEventDesc>),
    EmptyDescription(#[expect(dead_code)] Box<dyn TelemetryEventDesc>),
}

/// Checks that all telemetry events have a non-empty name and description.
///
/// The name and description are intended to be user-facing and are used to populate
/// our [exhaustive telemetry table](https://docs.warp.dev/support-and-community/privacy-and-security/privacy#exhaustive-telemetry-table).
#[test]
#[cfg(not(target_family = "wasm"))]
fn telemetry_events_have_nonempty_name_and_description() -> Result<(), TelemetryEventPropertyError>
{
    for event in warp_core::telemetry::all_events() {
        if event.name().is_empty() {
            return Err(TelemetryEventPropertyError::EmptyName(event));
        }
        if event.description().is_empty() {
            return Err(TelemetryEventPropertyError::EmptyDescription(event));
        }
    }
    Ok(())
}

#[test]
fn cloud_shell_recovery_detected_payload_is_bounded_and_non_ugc() {
    let event = TelemetryEvent::CloudAgentShellRecovery {
        outcome: CloudAgentShellRecoveryOutcome::Detected,
        attempt: 2,
        detection: CloudAgentShellExitDetection::Signal,
        status_available: true,
        exit_code: None,
        signal: Some(9),
        duration_ms: None,
        dynamic_session_environment_available: true,
        used_fallback_directory: None,
        failure_class: None,
    };

    assert_eq!(
        event.payload(),
        Some(json!({
            "outcome": "detected",
            "attempt": 2,
            "detection": "signal",
            "status_available": true,
            "exit_code": null,
            "signal": 9,
            "duration_ms": null,
            "dynamic_session_environment_available": true,
            "used_fallback_directory": null,
            "failure_class": null,
        }))
    );
    assert!(!event.contains_ugc());
}

#[test]
fn cloud_shell_recovery_failure_payload_has_classification_and_duration() {
    let event = TelemetryEvent::CloudAgentShellRecovery {
        outcome: CloudAgentShellRecoveryOutcome::Failed,
        attempt: 3,
        detection: CloudAgentShellExitDetection::Unavailable,
        status_available: false,
        exit_code: None,
        signal: None,
        duration_ms: Some(15_000),
        dynamic_session_environment_available: false,
        used_fallback_directory: Some(true),
        failure_class: Some(CloudAgentShellRecoveryFailureClass::BootstrapTimeout),
    };

    assert_eq!(
        event.payload(),
        Some(json!({
            "outcome": "failed",
            "attempt": 3,
            "detection": "unavailable",
            "status_available": false,
            "exit_code": null,
            "signal": null,
            "duration_ms": 15_000,
            "dynamic_session_environment_available": false,
            "used_fallback_directory": true,
            "failure_class": "bootstrap_timeout",
        }))
    );
    assert!(!event.contains_ugc());
}
