use settings::{Setting, SyncToCloud};

use super::{
    AllowInsideWarpControl, AllowInsideWarpReadOnly, AllowInsideWarpReadWrite,
    AllowOutsideWarpControl, AllowOutsideWarpReadOnly, AllowOutsideWarpReadWrite,
    LocalControlInvocationContext, LocalControlPermissionCategory, LocalControlSettings,
};

fn settings_with_values(
    inside_enabled: bool,
    outside_enabled: bool,
    inside_read_only: bool,
    outside_read_only: bool,
    inside_read_write: bool,
    outside_read_write: bool,
) -> LocalControlSettings {
    LocalControlSettings {
        allow_inside_warp_control: AllowInsideWarpControl::new(Some(inside_enabled)),
        allow_outside_warp_control: AllowOutsideWarpControl::new(Some(outside_enabled)),
        allow_inside_warp_read_only: AllowInsideWarpReadOnly::new(Some(inside_read_only)),
        allow_outside_warp_read_only: AllowOutsideWarpReadOnly::new(Some(outside_read_only)),
        allow_inside_warp_read_write: AllowInsideWarpReadWrite::new(Some(inside_read_write)),
        allow_outside_warp_read_write: AllowOutsideWarpReadWrite::new(Some(outside_read_write)),
    }
}

#[test]
fn defaults_allow_inside_warp_permissions_only() {
    let settings = settings_with_values(true, false, true, false, true, false);

    assert!(settings.allows(
        LocalControlInvocationContext::InsideWarp,
        LocalControlPermissionCategory::ReadOnly
    ));
    assert!(settings.allows(
        LocalControlInvocationContext::InsideWarp,
        LocalControlPermissionCategory::ReadWrite
    ));
    assert!(!settings.allows(
        LocalControlInvocationContext::OutsideWarp,
        LocalControlPermissionCategory::ReadOnly
    ));
    assert!(!settings.allows(
        LocalControlInvocationContext::OutsideWarp,
        LocalControlPermissionCategory::ReadWrite
    ));
}

#[test]
fn generated_settings_are_private_local_only_with_expected_defaults() {
    assert!(*AllowInsideWarpControl::new(None));
    assert!(!*AllowOutsideWarpControl::new(None));
    assert!(*AllowInsideWarpReadOnly::new(None));
    assert!(!*AllowOutsideWarpReadOnly::new(None));
    assert!(*AllowInsideWarpReadWrite::new(None));
    assert!(!*AllowOutsideWarpReadWrite::new(None));
    assert_eq!(AllowInsideWarpControl::sync_to_cloud(), SyncToCloud::Never);
    assert_eq!(AllowOutsideWarpControl::sync_to_cloud(), SyncToCloud::Never);
    assert_eq!(AllowInsideWarpReadOnly::sync_to_cloud(), SyncToCloud::Never);
    assert_eq!(
        AllowOutsideWarpReadWrite::sync_to_cloud(),
        SyncToCloud::Never
    );
    assert!(AllowInsideWarpControl::is_private());
    assert!(AllowOutsideWarpControl::is_private());
    assert!(AllowInsideWarpReadOnly::is_private());
    assert!(AllowOutsideWarpReadWrite::is_private());
}

#[test]
fn disabled_context_blocks_enabled_granular_permissions() {
    let settings = settings_with_values(false, false, true, true, true, true);

    assert!(!settings.allows(
        LocalControlInvocationContext::InsideWarp,
        LocalControlPermissionCategory::ReadWrite
    ));
    assert!(!settings.allows(
        LocalControlInvocationContext::OutsideWarp,
        LocalControlPermissionCategory::ReadOnly
    ));
}
