use super::should_collect_ai_ugc_telemetry_for_settings;
use crate::workspaces::workspace::{
    OrganizationTelemetryPolicy, TelemetryEnablementSetting, UgcCollectionEnablementSetting,
};
use crate::FeatureFlag;

#[test]
fn org_disable_is_authoritative() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _analytics_flag = FeatureFlag::GlobalAIAnalyticsCollection.override_enabled(true);
    let _agent_flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);
    assert!(!should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::Disable,
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Enabled),
        true,
        true,
    ));
}

#[test]
fn org_enable_is_authoritative_when_telemetry_is_on() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _agent_flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);
    assert!(should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::Enable,
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Enabled),
        false,
        false,
    ));
}

#[test]
fn enforced_disabled_telemetry_cascades_to_ugc() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _agent_flag = FeatureFlag::AgentModeAnalytics.override_enabled(true);
    assert!(!should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::Enable,
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Disabled),
        true,
        true,
    ));
}

#[test]
fn user_telemetry_opt_out_cascades_to_ugc_under_unmanaged_policy() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _agent_flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);
    assert!(!should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::Enable,
        OrganizationTelemetryPolicy::Unmanaged,
        false,
        true,
    ));
}

#[test]
fn respect_user_setting_requires_telemetry_and_ugc_preference() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _analytics_flag = FeatureFlag::GlobalAIAnalyticsCollection.override_enabled(true);
    let _agent_flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);
    assert!(should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Unmanaged,
        true,
        true,
    ));
    assert!(!should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Unmanaged,
        true,
        false,
    ));
    assert!(!should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Unmanaged,
        false,
        true,
    ));
}

#[test]
fn respect_user_setting_with_enforced_enabled_telemetry_uses_ugc_preference() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _analytics_flag = FeatureFlag::GlobalAIAnalyticsCollection.override_enabled(true);
    let _agent_flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);
    assert!(should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Enabled),
        false,
        true,
    ));
    assert!(!should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Enabled),
        false,
        false,
    ));
}

#[test]
fn rollout_off_preserves_legacy_behavior_plus_ugc_preference() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(false);
    let _analytics_flag = FeatureFlag::GlobalAIAnalyticsCollection.override_enabled(true);
    let _agent_flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);
    // No cascade when the rollout flag is off: the org UGC setting stays authoritative.
    assert!(should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::Enable,
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Disabled),
        false,
        false,
    ));
    assert!(should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Unmanaged,
        true,
        true,
    ));
    assert!(!should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Unmanaged,
        true,
        false,
    ));
    assert!(!should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Unmanaged,
        false,
        true,
    ));
}

#[test]
fn agent_mode_analytics_overrides_user_preferences_under_respect_user_setting() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _analytics_flag = FeatureFlag::GlobalAIAnalyticsCollection.override_enabled(false);
    let _agent_flag = FeatureFlag::AgentModeAnalytics.override_enabled(true);
    assert!(should_collect_ai_ugc_telemetry_for_settings(
        UgcCollectionEnablementSetting::RespectUserSetting,
        OrganizationTelemetryPolicy::Unmanaged,
        false,
        false,
    ));
}
