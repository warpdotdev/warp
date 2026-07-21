use serde_json::json;
use warp_core::features::FeatureFlag;
use warp_core::telemetry::TelemetryEvent;

use super::{ACCOUNT_FIRST_FLOW_VERSION, OnboardingEvent};

#[test]
fn account_first_started_payload_includes_flow_metadata() {
    let _account_first_onboarding = FeatureFlag::AccountFirstOnboarding.override_enabled(true);

    assert_eq!(
        OnboardingEvent::OnboardingStarted.payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "entrypoint": "native_app",
        }))
    );
}

#[test]
fn offer_action_payload_includes_account_class() {
    assert_eq!(
        OnboardingEvent::OnboardingAction {
            slide_name: "head_start".to_string(),
            action: "get_more_ai".to_string(),
            account_class: Some("free_icp".to_string()),
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "slide_name": "head_start",
            "action": "get_more_ai",
            "account_class": "free_icp",
        }))
    );
}

#[test]
fn account_first_slide_and_setting_payloads_include_flow_version() {
    let _account_first_onboarding = FeatureFlag::AccountFirstOnboarding.override_enabled(true);

    assert_eq!(
        OnboardingEvent::SlideViewed {
            slide_name: "customize".to_string(),
        }
        .payload(),
        Some(json!({
            "slide_name": "customize",
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
        }))
    );
    assert_eq!(
        OnboardingEvent::SettingChanged {
            setting: "theme".to_string(),
            value: "Dark".to_string(),
        }
        .payload(),
        Some(json!({
            "setting": "theme",
            "value": "Dark",
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
        }))
    );
}

#[test]
fn stable_slide_payload_does_not_include_flow_version() {
    let _account_first_onboarding = FeatureFlag::AccountFirstOnboarding.override_enabled(false);

    assert_eq!(OnboardingEvent::OnboardingStarted.payload(), None);
    assert_eq!(
        OnboardingEvent::SlideViewed {
            slide_name: "intro".to_string(),
        }
        .payload(),
        Some(json!({
            "slide_name": "intro",
        }))
    );
}

#[test]
fn onboarding_action_payload_omits_absent_account_class() {
    assert_eq!(
        OnboardingEvent::OnboardingAction {
            slide_name: "create_account".to_string(),
            action: "continue_signup".to_string(),
            account_class: None,
        }
        .payload(),
        Some(json!({
            "flow_version": ACCOUNT_FIRST_FLOW_VERSION,
            "slide_name": "create_account",
            "action": "continue_signup",
        }))
    );
}
