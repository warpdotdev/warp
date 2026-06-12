use warp_graphql::workspace::TelemetryEnabled as GqlTelemetryEnabled;

use super::organization_telemetry_policy;
use crate::features::FeatureFlag;
use crate::workspaces::workspace::{
    OrganizationTelemetryPolicy, TelemetryEnablementSetting as NativeTelemetryEnablementSetting,
};

#[test]
fn enable_setting_enforces_enabled() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::Enable, false),
        OrganizationTelemetryPolicy::Enforced(NativeTelemetryEnablementSetting::Enabled)
    );
}

#[test]
fn disable_setting_enforces_disabled() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::Disable, false),
        OrganizationTelemetryPolicy::Enforced(NativeTelemetryEnablementSetting::Disabled)
    );
}

#[test]
fn respect_user_setting_is_unmanaged() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::RespectUserSetting, false),
        OrganizationTelemetryPolicy::Unmanaged
    );
}

#[test]
fn setting_overrides_legacy_force_enabled() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::Disable, true),
        OrganizationTelemetryPolicy::Enforced(NativeTelemetryEnablementSetting::Disabled)
    );
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::RespectUserSetting, true),
        OrganizationTelemetryPolicy::Unmanaged
    );
}

#[test]
fn unknown_setting_falls_back_to_unmanaged() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(true);
    assert_eq!(
        organization_telemetry_policy(
            GqlTelemetryEnabled::Other("SOME_NEW_VALUE".to_string()),
            false,
        ),
        OrganizationTelemetryPolicy::Unmanaged
    );
}

#[test]
fn rollout_off_honors_enable_and_legacy_force_enabled_only() {
    let _flag = FeatureFlag::EnterpriseTelemetryPolicy.override_enabled(false);
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::Enable, false),
        OrganizationTelemetryPolicy::Enforced(NativeTelemetryEnablementSetting::Enabled)
    );
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::RespectUserSetting, true),
        OrganizationTelemetryPolicy::Enforced(NativeTelemetryEnablementSetting::Enabled)
    );
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::Disable, false),
        OrganizationTelemetryPolicy::Unmanaged
    );
    assert_eq!(
        organization_telemetry_policy(GqlTelemetryEnabled::RespectUserSetting, false),
        OrganizationTelemetryPolicy::Unmanaged
    );
    assert_eq!(
        organization_telemetry_policy(
            GqlTelemetryEnabled::Other("SOME_NEW_VALUE".to_string()),
            false,
        ),
        OrganizationTelemetryPolicy::Unmanaged
    );
}
