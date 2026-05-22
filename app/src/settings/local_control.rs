use settings::{macros::define_settings_group, SupportedPlatforms, SyncToCloud};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalControlInvocationContext {
    InsideWarp,
    OutsideWarp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalControlPermissionCategory {
    ReadOnly,
    ReadWrite,
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
    allow_inside_warp_read_only: AllowInsideWarpReadOnly {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlInsideWarpReadOnly",
        description: "Whether verified Warp-managed terminal sessions may receive read-only local control grants.",
    },
    allow_outside_warp_read_only: AllowOutsideWarpReadOnly {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlOutsideWarpReadOnly",
        description: "Whether external local clients may receive read-only local control grants.",
    },
    allow_inside_warp_read_write: AllowInsideWarpReadWrite {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlInsideWarpReadWrite",
        description: "Whether verified Warp-managed terminal sessions may receive read-write local control grants.",
    },
    allow_outside_warp_read_write: AllowOutsideWarpReadWrite {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "LocalControlOutsideWarpReadWrite",
        description: "Whether external local clients may receive read-write local control grants.",
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
                LocalControlPermissionCategory::ReadOnly,
            ) => *self.allow_inside_warp_read_only,
            (
                LocalControlInvocationContext::OutsideWarp,
                LocalControlPermissionCategory::ReadOnly,
            ) => *self.allow_outside_warp_read_only,
            (
                LocalControlInvocationContext::InsideWarp,
                LocalControlPermissionCategory::ReadWrite,
            ) => *self.allow_inside_warp_read_write,
            (
                LocalControlInvocationContext::OutsideWarp,
                LocalControlPermissionCategory::ReadWrite,
            ) => *self.allow_outside_warp_read_write,
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
