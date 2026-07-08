use settings::macros::define_settings_group;
use settings::{RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud};
use warpui::{AppContext, SingletonEntity};

use crate::terminal::model::ObfuscateSecrets;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// How secrets should be displayed in the block list
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "How detected secrets are visually displayed.",
    rename_all = "snake_case"
)]
pub enum SecretDisplayMode {
    /// Fully obscure secrets with asterisks
    Asterisks,
    /// Show secrets with gray color and strikethrough styling
    #[default]
    Strikethrough,
    /// Show secrets normally with no visual treatment (but are still detected/redacted)
    AlwaysShow,
}

impl SecretDisplayMode {
    /// Convert to the corresponding ObfuscateSecrets enum for visual rendering
    pub fn to_obfuscate_secrets(self) -> ObfuscateSecrets {
        match self {
            SecretDisplayMode::Asterisks => ObfuscateSecrets::Yes,
            SecretDisplayMode::Strikethrough => ObfuscateSecrets::Strikethrough,
            SecretDisplayMode::AlwaysShow => ObfuscateSecrets::AlwaysShow,
        }
    }

    /// Convert from legacy boolean setting for backward compatibility
    pub fn from_legacy_hide_secrets(hide_secrets: bool) -> Self {
        if hide_secrets {
            SecretDisplayMode::Asterisks
        } else {
            SecretDisplayMode::Strikethrough
        }
    }

    /// Display name for UI
    pub fn display_name(self) -> &'static str {
        match self {
            SecretDisplayMode::Asterisks => "Asterisks",
            SecretDisplayMode::Strikethrough => "Strikethrough",
            SecretDisplayMode::AlwaysShow => "Always show secrets",
        }
    }

    /// Get all available modes for dropdown
    pub fn all_modes() -> [SecretDisplayMode; 3] {
        [
            SecretDisplayMode::Asterisks,
            SecretDisplayMode::Strikethrough,
            SecretDisplayMode::AlwaysShow,
        ]
    }
}

define_settings_group!(SafeModeSettings, settings: [
    safe_mode_enabled: SafeModeEnabled {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "privacy.secret_redaction.enabled",
        description: "Whether secret redaction is enabled to detect and obscure secrets in terminal output.",
    },
    secret_display_mode: SecretDisplayModeSetting {
        type: SecretDisplayMode,
        default: SecretDisplayMode::Strikethrough,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "privacy.secret_redaction.secret_display_mode_setting",
        description: "Controls how detected secrets are visually displayed in the terminal.",
    },
    // Keep legacy setting for backward compatibility during migration
    hide_secrets_in_block_list: HideSecretsInBlockList {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        surface: settings::SettingSurfaces::GUI,
        private: false,
        toml_path: "privacy.secret_redaction.hide_secrets_in_block_list",
        description: "Whether to hide detected secrets in the block list using asterisks.",
    },
]);

/// Returns whether the rendering should obfuscate secrets given the current safe mode settings.
pub fn get_secret_obfuscation_mode(app: &AppContext) -> ObfuscateSecrets {
    let safe_mode_settings = SafeModeSettings::as_ref(app);
    let is_enterprise_secret_redaction_enabled =
        UserWorkspaces::as_ref(app).is_enterprise_secret_redaction_enabled();

    if !is_enterprise_secret_redaction_enabled && !*safe_mode_settings.safe_mode_enabled.value() {
        ObfuscateSecrets::No
    } else {
        let mode = get_effective_secret_display_mode(safe_mode_settings);
        mode.to_obfuscate_secrets()
    }
}

/// Get the effective secret display mode, handling backward compatibility
pub fn get_effective_secret_display_mode(
    safe_mode_settings: &SafeModeSettings,
) -> SecretDisplayMode {
    // Check if user has migrated to new setting (non-default value or explicit setting)
    let current_mode = *safe_mode_settings.secret_display_mode.value();
    let legacy_hide_secrets = *safe_mode_settings.hide_secrets_in_block_list.value();

    // If the new setting is at default and legacy setting is non-default, migrate
    if current_mode == SecretDisplayMode::default() && legacy_hide_secrets {
        SecretDisplayMode::from_legacy_hide_secrets(legacy_hide_secrets)
    } else {
        current_mode
    }
}
