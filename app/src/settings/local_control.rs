//! Secure local setting that gates local-control invocation contexts.
//!
//! This setting is local-only, kept out of the user-visible settings file, and
//! persisted through Warp's secure storage provider. It is the authoritative
//! enablement bit for local control.
use anyhow::Result;
use serde::{Deserialize, Serialize};
use settings::macros::define_settings_group;
use settings::{SecureSetting, Setting, SupportedPlatforms, SyncToCloud};
use warpui::{AppContext, ModelContext};
use warpui_extras::secure_storage;

const LOCAL_CONTROL_MODE_STORAGE_KEY: &str = "LocalControlMode";

/// User-selected local-control availability.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Deserialize,
    Eq,
    PartialEq,
    schemars::JsonSchema,
    Serialize,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Which local-control invocation contexts are allowed.",
    rename_all = "snake_case"
)]
pub enum LocalControlMode {
    #[default]
    Disabled,
    EnabledWithinWarp,
    EnabledEverywhere,
}

impl LocalControlMode {
    pub const ALL: [Self; 3] = [
        Self::Disabled,
        Self::EnabledWithinWarp,
        Self::EnabledEverywhere,
    ];

    pub fn allows_inside_warp(self) -> bool {
        matches!(self, Self::EnabledWithinWarp | Self::EnabledEverywhere)
    }

    pub fn allows_outside_warp(self) -> bool {
        matches!(self, Self::EnabledEverywhere)
    }

    pub fn as_dropdown_label(self) -> &'static str {
        match self {
            Self::Disabled => "Disabled",
            Self::EnabledWithinWarp => "Enabled within Warp",
            Self::EnabledEverywhere => "Enabled everywhere, including outside Warp",
        }
    }
}

define_settings_group!(LocalControlSettings, settings: [
    local_control_mode: LocalControlModeSetting,
]);

/// Setting wrapper for the authoritative local-control mode.
pub struct LocalControlModeSetting {
    inner: LocalControlMode,
    is_explicitly_set: bool,
}

impl LocalControlModeSetting {
    fn emit_changed(
        ctx: &mut ModelContext<LocalControlSettings>,
        change_event_reason: settings::ChangeEventReason,
    ) {
        ctx.emit(LocalControlSettingsChangedEvent::LocalControlModeSetting {
            change_event_reason,
        });
    }
}

impl SecureSetting for LocalControlModeSetting {
    fn write_secure_storage_value(
        storage: &dyn secure_storage::SecureStorage,
        key: &str,
        value: &str,
    ) -> Result<(), secure_storage::Error> {
        storage.write_value_with_owner_only_fallback(key, value)
    }
}
impl Setting for LocalControlModeSetting {
    type Group = LocalControlSettings;
    type Value = LocalControlMode;

    fn new(value: Option<Self::Value>) -> Self {
        match value {
            Some(value) => Self {
                inner: value,
                is_explicitly_set: true,
            },
            None => Self {
                inner: Self::default_value(),
                is_explicitly_set: false,
            },
        }
    }

    fn setting_name() -> &'static str {
        "LocalControlModeSetting"
    }

    fn storage_key() -> &'static str {
        LOCAL_CONTROL_MODE_STORAGE_KEY
    }

    fn supported_platforms() -> SupportedPlatforms {
        SupportedPlatforms::DESKTOP
    }

    fn sync_to_cloud() -> SyncToCloud {
        SyncToCloud::Never
    }

    fn is_private() -> bool {
        true
    }

    fn value(&self) -> &Self::Value {
        &self.inner
    }

    fn clear_value(&mut self, ctx: &mut ModelContext<Self::Group>) -> Result<()> {
        Self::clear_from_secure_storage(ctx)?;
        self.inner = self.validate(Self::default_value());
        self.is_explicitly_set = false;
        Self::emit_changed(ctx, settings::ChangeEventReason::Clear);
        Ok(())
    }

    fn load_value(
        &mut self,
        new_value: Self::Value,
        explicitly_set: bool,
        ctx: &mut ModelContext<Self::Group>,
    ) -> Result<()> {
        let validated = self.validate(new_value);
        if self.value() != &validated || self.is_explicitly_set != explicitly_set {
            self.inner = validated;
            self.is_explicitly_set = explicitly_set;
            Self::emit_changed(ctx, settings::ChangeEventReason::LocalChange);
        }
        Ok(())
    }

    fn set_value_from_cloud_sync(
        &mut self,
        _: Self::Value,
        _: &mut ModelContext<Self::Group>,
    ) -> Result<()> {
        Ok(())
    }

    fn set_value(
        &mut self,
        new_value: Self::Value,
        ctx: &mut ModelContext<Self::Group>,
    ) -> Result<()> {
        let changed_in_storage = Self::write_to_secure_storage(&new_value, ctx)?;
        if self.value() != &new_value || changed_in_storage {
            self.inner = self.validate(new_value);
            self.is_explicitly_set = true;
            Self::emit_changed(ctx, settings::ChangeEventReason::LocalChange);
        }
        Ok(())
    }

    fn default_value() -> Self::Value {
        LocalControlMode::Disabled
    }

    fn new_from_storage(ctx: &mut AppContext) -> Self {
        Self::new(Self::read_from_secure_storage(ctx))
    }

    fn is_supported_on_current_platform(&self) -> bool {
        SupportedPlatforms::DESKTOP.matches_current_platform()
    }

    fn is_value_explicitly_set(&self) -> bool {
        self.is_explicitly_set
    }
}

impl std::ops::Deref for LocalControlModeSetting {
    type Target = LocalControlMode;

    fn deref(&self) -> &Self::Target {
        self.value()
    }
}

impl LocalControlSettings {
    pub fn mode(&self) -> LocalControlMode {
        *self.local_control_mode
    }

    pub fn inside_warp_control_enabled(&self) -> bool {
        self.mode().allows_inside_warp()
    }

    pub fn outside_warp_control_enabled(&self) -> bool {
        self.mode().allows_outside_warp()
    }
}

#[cfg(test)]
#[path = "local_control_tests.rs"]
mod tests;
