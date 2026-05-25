use settings::{Setting, SyncToCloud};

use super::{
    AllowOutsideWarpAppStateMutations, AllowOutsideWarpControl,
    AllowOutsideWarpMetadataConfigurationMutations, AllowOutsideWarpMetadataReads,
    AllowOutsideWarpUnderlyingDataMutations, AllowOutsideWarpUnderlyingDataReads,
    LocalControlPermissionCategory, LocalControlSettings,
};

fn settings_with_values(outside_enabled: bool) -> LocalControlSettings {
    LocalControlSettings {
        allow_outside_warp_control: AllowOutsideWarpControl::new(Some(outside_enabled)),
        allow_outside_warp_metadata_reads: AllowOutsideWarpMetadataReads::new(Some(false)),
        allow_outside_warp_underlying_data_reads: AllowOutsideWarpUnderlyingDataReads::new(Some(
            false,
        )),
        allow_outside_warp_app_state_mutations: AllowOutsideWarpAppStateMutations::new(Some(false)),
        allow_outside_warp_metadata_configuration_mutations:
            AllowOutsideWarpMetadataConfigurationMutations::new(Some(false)),
        allow_outside_warp_underlying_data_mutations: AllowOutsideWarpUnderlyingDataMutations::new(
            Some(false),
        ),
    }
}

#[test]
fn defaults_disable_outside_warp_permissions() {
    let settings = settings_with_values(false);

    for permission in [
        LocalControlPermissionCategory::MetadataReads,
        LocalControlPermissionCategory::UnderlyingDataReads,
        LocalControlPermissionCategory::AppStateMutations,
        LocalControlPermissionCategory::MetadataConfigurationMutations,
        LocalControlPermissionCategory::UnderlyingDataMutations,
    ] {
        assert!(!settings.allows_outside_warp(permission));
    }
}

#[test]
fn generated_settings_are_private_local_only_with_expected_defaults() {
    assert!(!*AllowOutsideWarpControl::new(None));
    assert!(!*AllowOutsideWarpMetadataReads::new(None));
    assert!(!*AllowOutsideWarpUnderlyingDataReads::new(None));
    assert!(!*AllowOutsideWarpAppStateMutations::new(None));
    assert!(!*AllowOutsideWarpMetadataConfigurationMutations::new(None));
    assert!(!*AllowOutsideWarpUnderlyingDataMutations::new(None));
    assert_eq!(AllowOutsideWarpControl::sync_to_cloud(), SyncToCloud::Never);
    assert_eq!(
        AllowOutsideWarpUnderlyingDataMutations::sync_to_cloud(),
        SyncToCloud::Never
    );
    assert!(AllowOutsideWarpControl::is_private());
    assert!(AllowOutsideWarpMetadataReads::is_private());
    assert!(AllowOutsideWarpUnderlyingDataMutations::is_private());
}

#[test]
fn disabled_context_blocks_enabled_granular_permissions() {
    let settings = LocalControlSettings {
        allow_outside_warp_control: AllowOutsideWarpControl::new(Some(false)),
        allow_outside_warp_metadata_reads: AllowOutsideWarpMetadataReads::new(Some(true)),
        allow_outside_warp_underlying_data_reads: AllowOutsideWarpUnderlyingDataReads::new(Some(
            true,
        )),
        allow_outside_warp_app_state_mutations: AllowOutsideWarpAppStateMutations::new(Some(true)),
        allow_outside_warp_metadata_configuration_mutations:
            AllowOutsideWarpMetadataConfigurationMutations::new(Some(true)),
        allow_outside_warp_underlying_data_mutations: AllowOutsideWarpUnderlyingDataMutations::new(
            Some(true),
        ),
    };

    assert!(!settings.allows_outside_warp(LocalControlPermissionCategory::AppStateMutations));
    assert!(!settings.allows_outside_warp(LocalControlPermissionCategory::MetadataReads));
}

#[test]
fn granular_permissions_are_independent() {
    let settings = LocalControlSettings {
        allow_outside_warp_control: AllowOutsideWarpControl::new(Some(true)),
        allow_outside_warp_metadata_reads: AllowOutsideWarpMetadataReads::new(Some(true)),
        allow_outside_warp_underlying_data_reads: AllowOutsideWarpUnderlyingDataReads::new(Some(
            false,
        )),
        allow_outside_warp_app_state_mutations: AllowOutsideWarpAppStateMutations::new(Some(true)),
        allow_outside_warp_metadata_configuration_mutations:
            AllowOutsideWarpMetadataConfigurationMutations::new(Some(false)),
        allow_outside_warp_underlying_data_mutations: AllowOutsideWarpUnderlyingDataMutations::new(
            Some(false),
        ),
    };

    assert!(settings.allows_outside_warp(LocalControlPermissionCategory::MetadataReads));
    assert!(!settings.allows_outside_warp(LocalControlPermissionCategory::UnderlyingDataReads));
    assert!(settings.allows_outside_warp(LocalControlPermissionCategory::AppStateMutations));
    assert!(!settings
        .allows_outside_warp(LocalControlPermissionCategory::MetadataConfigurationMutations));
}
