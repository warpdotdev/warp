//! Private local settings that gate Warp control by invocation context and risk category.
//!
//! These settings are local-only and kept out of the user-visible settings file,
//! but this foundation branch still stores them in the existing private
//! preferences backend. Before outside-Warp control or broader grants ship,
//! the authoritative enablement bits should move to protected storage where
//! available, such as macOS Keychain or the platform equivalent, so external
//! apps cannot enable local control by editing ordinary preferences.
use settings::{macros::define_settings_group, SupportedPlatforms, SyncToCloud};

/// Source context for a local-control request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalControlInvocationContext {
    InsideWarp,
    OutsideWarp,
}

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
    allow_inside_warp_control: AllowInsideWarpControl {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlAllowInsideWarp",
        description: "Whether Warp control is allowed from verified Warp-managed terminal sessions.",
    },
    allow_outside_warp_control: AllowOutsideWarpControl {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlAllowOutsideWarp",
        description: "Whether Warp control is allowed from external local clients.",
    },
    allow_inside_warp_metadata_reads: AllowInsideWarpMetadataReads {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlInsideWarpMetadataReads",
        description: "Whether verified Warp-managed terminal sessions may receive metadata-read local control grants.",
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
    allow_inside_warp_underlying_data_reads: AllowInsideWarpUnderlyingDataReads {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlInsideWarpUnderlyingDataReads",
        description: "Whether verified Warp-managed terminal sessions may receive underlying-data-read local control grants.",
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
    allow_inside_warp_app_state_mutations: AllowInsideWarpAppStateMutations {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlInsideWarpAppStateMutations",
        description: "Whether verified Warp-managed terminal sessions may receive app-state-mutation local control grants.",
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
    allow_inside_warp_metadata_configuration_mutations: AllowInsideWarpMetadataConfigurationMutations {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlInsideWarpMetadataConfigurationMutations",
        description: "Whether verified Warp-managed terminal sessions may receive metadata/configuration-mutation local control grants.",
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
    allow_inside_warp_underlying_data_mutations: AllowInsideWarpUnderlyingDataMutations {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlInsideWarpUnderlyingDataMutations",
        description: "Whether verified Warp-managed terminal sessions may receive underlying-data-mutation local control grants.",
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
    pub fn is_context_enabled(&self, context: LocalControlInvocationContext) -> bool {
        match context {
            LocalControlInvocationContext::InsideWarp => *self.allow_inside_warp_control,
            LocalControlInvocationContext::OutsideWarp => *self.allow_outside_warp_control,
        }
    }

    pub fn is_permission_enabled(
        &self,
        context: LocalControlInvocationContext,
        permission: LocalControlPermissionCategory,
    ) -> bool {
        match (context, permission) {
            (
                LocalControlInvocationContext::InsideWarp,
                LocalControlPermissionCategory::MetadataReads,
            ) => *self.allow_inside_warp_metadata_reads,
            (
                LocalControlInvocationContext::OutsideWarp,
                LocalControlPermissionCategory::MetadataReads,
            ) => *self.allow_outside_warp_metadata_reads,
            (
                LocalControlInvocationContext::InsideWarp,
                LocalControlPermissionCategory::UnderlyingDataReads,
            ) => *self.allow_inside_warp_underlying_data_reads,
            (
                LocalControlInvocationContext::OutsideWarp,
                LocalControlPermissionCategory::UnderlyingDataReads,
            ) => *self.allow_outside_warp_underlying_data_reads,
            (
                LocalControlInvocationContext::InsideWarp,
                LocalControlPermissionCategory::AppStateMutations,
            ) => *self.allow_inside_warp_app_state_mutations,
            (
                LocalControlInvocationContext::OutsideWarp,
                LocalControlPermissionCategory::AppStateMutations,
            ) => *self.allow_outside_warp_app_state_mutations,
            (
                LocalControlInvocationContext::InsideWarp,
                LocalControlPermissionCategory::MetadataConfigurationMutations,
            ) => *self.allow_inside_warp_metadata_configuration_mutations,
            (
                LocalControlInvocationContext::OutsideWarp,
                LocalControlPermissionCategory::MetadataConfigurationMutations,
            ) => *self.allow_outside_warp_metadata_configuration_mutations,
            (
                LocalControlInvocationContext::InsideWarp,
                LocalControlPermissionCategory::UnderlyingDataMutations,
            ) => *self.allow_inside_warp_underlying_data_mutations,
            (
                LocalControlInvocationContext::OutsideWarp,
                LocalControlPermissionCategory::UnderlyingDataMutations,
            ) => *self.allow_outside_warp_underlying_data_mutations,
        }
    }

    pub fn allows(
        &self,
        context: LocalControlInvocationContext,
        permission: LocalControlPermissionCategory,
    ) -> bool {
        self.is_context_enabled(context) && self.is_permission_enabled(context, permission)
    }
}

#[cfg(test)]
#[path = "local_control_tests.rs"]
mod tests;
