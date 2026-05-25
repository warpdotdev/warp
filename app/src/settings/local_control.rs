//! Private local settings that gate outside-Warp control by risk category.
//!
//! These settings are local-only and kept out of the user-visible settings file,
//! but this foundation branch still stores them in the existing private
//! preferences backend. Before outside-Warp control ships, the authoritative
//! enablement bits should move to protected storage where available, such as
//! macOS Keychain or the platform equivalent, so external apps cannot enable
//! local control by editing ordinary preferences.
use settings::{macros::define_settings_group, SupportedPlatforms, SyncToCloud};

/// Coarse permission buckets used to gate groups of control actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalControlPermissionCategory {
    MetadataReads,
    UnderlyingDataReads,
    AppStateMutations,
    MetadataConfigurationMutations,
    UnderlyingDataMutations,
}

define_settings_group!(LocalControlSettings, settings: [
    allow_outside_warp_control: AllowOutsideWarpControl {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlAllowOutsideWarp",
        description: "Whether Warp control is allowed from external local clients.",
    },
    allow_outside_warp_metadata_reads: AllowOutsideWarpMetadataReads {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlOutsideWarpMetadataReads",
        description: "Whether external local clients may receive metadata-read local control grants.",
    },
    allow_outside_warp_underlying_data_reads: AllowOutsideWarpUnderlyingDataReads {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlOutsideWarpUnderlyingDataReads",
        description: "Whether external local clients may receive underlying-data-read local control grants.",
    },
    allow_outside_warp_app_state_mutations: AllowOutsideWarpAppStateMutations {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlOutsideWarpAppStateMutations",
        description: "Whether external local clients may receive app-state-mutation local control grants.",
    },
    allow_outside_warp_metadata_configuration_mutations: AllowOutsideWarpMetadataConfigurationMutations {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlOutsideWarpMetadataConfigurationMutations",
        description: "Whether external local clients may receive metadata/configuration-mutation local control grants.",
    },
    allow_outside_warp_underlying_data_mutations: AllowOutsideWarpUnderlyingDataMutations {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlOutsideWarpUnderlyingDataMutations",
        description: "Whether external local clients may receive underlying-data-mutation local control grants.",
    },
]);

impl LocalControlSettings {
    pub fn outside_warp_control_enabled(&self) -> bool {
        *self.allow_outside_warp_control
    }

    pub fn outside_warp_permission_enabled(
        &self,
        permission: LocalControlPermissionCategory,
    ) -> bool {
        match permission {
            LocalControlPermissionCategory::MetadataReads => {
                *self.allow_outside_warp_metadata_reads
            }
            LocalControlPermissionCategory::UnderlyingDataReads => {
                *self.allow_outside_warp_underlying_data_reads
            }
            LocalControlPermissionCategory::AppStateMutations => {
                *self.allow_outside_warp_app_state_mutations
            }
            LocalControlPermissionCategory::MetadataConfigurationMutations => {
                *self.allow_outside_warp_metadata_configuration_mutations
            }
            LocalControlPermissionCategory::UnderlyingDataMutations => {
                *self.allow_outside_warp_underlying_data_mutations
            }
        }
    }

    pub fn allows_outside_warp(&self, permission: LocalControlPermissionCategory) -> bool {
        self.outside_warp_control_enabled() && self.outside_warp_permission_enabled(permission)
    }
}

#[cfg(test)]
#[path = "local_control_tests.rs"]
mod tests;
