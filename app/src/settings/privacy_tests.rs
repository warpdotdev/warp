use warpui::App;

use super::*;

fn snapshot(policy: OrganizationTelemetryPolicy, user_enabled: bool) -> PrivacySettingsSnapshot {
    PrivacySettingsSnapshot {
        is_telemetry_enabled: user_enabled,
        is_crash_reporting_enabled: true,
        organization_telemetry_policy: policy,
        should_collect_ai_ugc_telemetry: true,
        cloud_conversation_storage_enabled: None,
        ugc_collection_enabled: None,
    }
}

#[test]
fn unresolved_policy_defaults_to_respecting_user_setting() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _analytics_flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);
    assert!(!snapshot(OrganizationTelemetryPolicy::default(), true).should_disable_telemetry());
    assert!(snapshot(OrganizationTelemetryPolicy::default(), false).should_disable_telemetry());
}

#[test]
fn enforced_disabled_overrides_user_and_agent_analytics() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    let _flag = FeatureFlag::AgentModeAnalytics.override_enabled(true);
    assert!(snapshot(
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Disabled),
        true,
    )
    .should_disable_telemetry());
}

#[test]
fn rollout_off_normalizes_disabled_policy_to_unmanaged() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(false);
    App::test((), |mut app| async move {
        app.add_singleton_model(PrivacySettings::mock);

        PrivacySettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.set_organization_telemetry_policy(
                OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Disabled),
                ctx,
            );
        });

        app.read(|ctx| {
            assert_eq!(
                PrivacySettings::as_ref(ctx).organization_telemetry_policy(),
                OrganizationTelemetryPolicy::Unmanaged
            );
        });
    });
}

#[test]
fn enforced_enabled_overrides_user_opt_out() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    assert!(!snapshot(
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Enabled),
        false,
    )
    .should_disable_telemetry());
}

#[test]
fn unmanaged_preserves_user_preference_and_agent_analytics_override() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    {
        let _flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);
        assert!(snapshot(OrganizationTelemetryPolicy::Unmanaged, false).should_disable_telemetry());
        assert!(!snapshot(OrganizationTelemetryPolicy::Unmanaged, true).should_disable_telemetry());
    }

    let _flag = FeatureFlag::AgentModeAnalytics.override_enabled(true);
    assert!(!snapshot(OrganizationTelemetryPolicy::Unmanaged, false).should_disable_telemetry());
}

#[test]
fn rollout_off_ignores_disabled_but_preserves_legacy_enabled() {
    let _policy_flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(false);
    let _analytics_flag = FeatureFlag::AgentModeAnalytics.override_enabled(false);

    assert!(!snapshot(
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Disabled),
        true,
    )
    .should_disable_telemetry());
    assert!(!snapshot(
        OrganizationTelemetryPolicy::Enforced(TelemetryEnablementSetting::Enabled),
        false,
    )
    .should_disable_telemetry());
}
