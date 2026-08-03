use anyhow::anyhow;
use serde_json::json;
use warp_core::telemetry::TelemetryEvent as _;

use super::{
    AbandonmentPhase, AuthenticationAttempt, AuthenticationEntrypoint, AuthenticationFailureReason,
    AuthenticationFailureStage, BrowserLaunchTrigger, Journey, Outcome, TuiOnboardingTelemetry,
    TuiOnboardingTelemetryEvent,
};
use crate::server::server_api::auth::UserAuthenticationError;

#[test]
fn event_names_and_payloads_are_stable() {
    let events = [
        (
            TuiOnboardingTelemetryEvent::AuthenticationStarted {
                journey: Journey::InitialLogin,
                entrypoint: AuthenticationEntrypoint::CopyUrl,
                attempt: AuthenticationAttempt::Retry,
            },
            "TUI.Onboarding.AuthenticationStarted",
            Some(json!({
                "journey": "initial_login",
                "entrypoint": "copy_url",
                "attempt": "retry",
            })),
        ),
        (
            TuiOnboardingTelemetryEvent::DeviceAuthorizationReady,
            "TUI.Onboarding.DeviceAuthorizationReady",
            None,
        ),
        (
            TuiOnboardingTelemetryEvent::BrowserLaunch {
                journey: Journey::PostLogout,
                trigger: BrowserLaunchTrigger::PostLogout,
                outcome: Outcome::Failed,
            },
            "TUI.Onboarding.BrowserLaunch",
            Some(json!({
                "journey": "post_logout",
                "trigger": "post_logout",
                "outcome": "failed",
            })),
        ),
        (
            TuiOnboardingTelemetryEvent::LoginUrlCopied {
                outcome: Outcome::Succeeded,
            },
            "TUI.Onboarding.LoginUrlCopied",
            Some(json!({ "outcome": "succeeded" })),
        ),
        (
            TuiOnboardingTelemetryEvent::AuthenticationFailed {
                journey: Journey::InitialLogin,
                stage: AuthenticationFailureStage::Authentication,
                reason: AuthenticationFailureReason::InvalidState,
                duration_ms: 42,
            },
            "TUI.Onboarding.AuthenticationFailed",
            Some(json!({
                "journey": "initial_login",
                "stage": "authentication",
                "reason": "invalid_state",
                "duration_ms": 42,
            })),
        ),
        (
            TuiOnboardingTelemetryEvent::Abandoned {
                journey: Journey::InitialLogin,
                phase: AbandonmentPhase::BrowserOpenFailed,
                duration_ms: 43,
            },
            "TUI.Onboarding.Abandoned",
            Some(json!({
                "journey": "initial_login",
                "phase": "browser_open_failed",
                "duration_ms": 43,
            })),
        ),
        (
            TuiOnboardingTelemetryEvent::Completed {
                journey: Journey::PostLogout,
                duration_ms: 44,
            },
            "TUI.Onboarding.Completed",
            Some(json!({
                "journey": "post_logout",
                "duration_ms": 44,
            })),
        ),
    ];

    for (event, expected_name, expected_payload) in events {
        assert_eq!(event.name(), expected_name);
        assert_eq!(event.payload(), expected_payload);
        assert!(!event.contains_ugc());
    }
}

#[test]
fn flow_tracks_retries_and_deduplicates_terminal_outcomes() {
    let mut telemetry = TuiOnboardingTelemetry::new(false);

    assert!(matches!(
        telemetry.authentication_started(AuthenticationEntrypoint::OpenBrowser),
        TuiOnboardingTelemetryEvent::AuthenticationStarted {
            journey: Journey::InitialLogin,
            attempt: AuthenticationAttempt::Initial,
            ..
        }
    ));
    assert_eq!(
        telemetry.device_authorization_ready(),
        Some(TuiOnboardingTelemetryEvent::DeviceAuthorizationReady)
    );
    assert_eq!(telemetry.device_authorization_ready(), None);
    assert!(matches!(
        telemetry.authentication_failed(&UserAuthenticationError::Unexpected(anyhow!(
            "raw detail that must not enter telemetry"
        ))),
        Some(TuiOnboardingTelemetryEvent::AuthenticationFailed {
            stage: AuthenticationFailureStage::Authentication,
            reason: AuthenticationFailureReason::Unexpected,
            ..
        })
    ));

    assert!(matches!(
        telemetry.authentication_started(AuthenticationEntrypoint::CopyUrl),
        TuiOnboardingTelemetryEvent::AuthenticationStarted {
            attempt: AuthenticationAttempt::Retry,
            ..
        }
    ));
    assert!(matches!(
        telemetry.browser_launch(true),
        Some(TuiOnboardingTelemetryEvent::BrowserLaunch {
            trigger: BrowserLaunchTrigger::Retry,
            outcome: Outcome::Succeeded,
            ..
        })
    ));
    assert!(matches!(
        telemetry.completed(),
        Some(TuiOnboardingTelemetryEvent::Completed {
            journey: Journey::InitialLogin,
            ..
        })
    ));
    assert_eq!(telemetry.completed(), None);
    assert_eq!(telemetry.abandoned(AbandonmentPhase::WaitingForLogin), None);
}

#[test]
fn post_logout_failure_uses_low_cardinality_device_request_dimensions() {
    let mut telemetry = TuiOnboardingTelemetry::new(true);

    assert!(matches!(
        telemetry.post_logout_authentication_started(),
        TuiOnboardingTelemetryEvent::AuthenticationStarted {
            journey: Journey::PostLogout,
            attempt: AuthenticationAttempt::Initial,
            ..
        }
    ));
    assert!(matches!(
        telemetry.authentication_failed(&UserAuthenticationError::DeviceCodeRequestTimedOut {
            attempts: 2
        }),
        Some(TuiOnboardingTelemetryEvent::AuthenticationFailed {
            journey: Journey::PostLogout,
            stage: AuthenticationFailureStage::DeviceCodeRequest,
            reason: AuthenticationFailureReason::DeviceCodeRequestTimeout,
            ..
        })
    ));
}
