use std::fmt::Display;
use std::sync::Arc;

use anyhow::Result;
use regex::Regex;
use warp_core::features::FeatureFlag;
use warp_core::report_if_error;
use warpui::{AppContext, Entity, ModelContext, SingletonEntity, UpdateModel};

use crate::ai::blocklist::telemetry_banner::should_collect_ai_ugc_telemetry;
use crate::auth::auth_state::AuthState;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::report_error;
use crate::server::cloud_objects::update_manager::UpdateManager;
#[cfg(test)]
use crate::server::server_api::auth::MockAuthClient;
use crate::server::server_api::auth::{AuthClient, SyncedUserSettings};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::safe_mode_settings::SafeModeSettings;

use settings::{
    macros::{define_settings_group, maybe_define_setting, register_settings_events},
    RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};

use serde::{Deserialize, Serialize};

use super::cloud_preferences_syncer::CloudPreferencesSyncer;
use crate::workspaces::workspace::EnterpriseSecretRegex;

pub trait RegexDisplayInfo {
    fn pattern(&self) -> &str;
    fn name(&self) -> Option<&str>;
}

pub const TELEMETRY_ENABLED_DEFAULTS_KEY: &str = "TelemetryEnabled";
pub const CRASH_REPORTING_ENABLED_DEFAULTS_KEY: &str = "CrashReportingEnabled";
pub const CLOUD_CONVERSATION_STORAGE_ENABLED_DEFAULTS_KEY: &str = "CloudConversationStorageEnabled";

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(description = "A custom regex pattern for detecting and redacting secrets.")]
pub struct CustomSecretRegex {
    #[serde(with = "serde_regex")]
    #[schemars(with = "String", description = "The regex pattern to match secrets.")]
    pub pattern: Regex,
    #[serde(default)]
    #[schemars(description = "Optional display name for this secret pattern.")]
    pub name: Option<String>,
}

impl CustomSecretRegex {
    pub fn pattern(&self) -> &Regex {
        &self.pattern
    }
}

impl RegexDisplayInfo for CustomSecretRegex {
    fn pattern(&self) -> &str {
        self.pattern.as_str()
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl RegexDisplayInfo for EnterpriseSecretRegex {
    fn pattern(&self) -> &str {
        &self.pattern
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl Display for CustomSecretRegex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.pattern.as_str())
    }
}

impl PartialEq for CustomSecretRegex {
    /// We do not factor in the name to equality checks --
    /// if the regex is the same, then the regex is the same.
    /// This allows us to avoid adding duplicate regexes.
    fn eq(&self, other: &Self) -> bool {
        self.pattern.as_str() == other.pattern.as_str()
    }
}

impl settings_value::SettingsValue for CustomSecretRegex {}

define_settings_group!(WarpDrivePrivacySettings, settings: [
    is_telemetry_enabled: IsTelemetryEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: false,
        storage_key: "TelemetryEnabled",
        toml_path: "privacy.telemetry_enabled",
        description: "Whether anonymous usage telemetry is collected.",
    },
    is_crash_reporting_enabled: IsCrashReportingEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: false,
        storage_key: "CrashReportingEnabled",
        toml_path: "privacy.crash_reporting_enabled",
        description: "Whether crash and internal error reports are sent.",
    },
    is_cloud_conversation_storage_enabled: IsCloudConversationStorageEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
        private: false,
        storage_key: "CloudConversationStorageEnabled",
        toml_path: "agents.cloud_conversation_storage_enabled",
        description: "Whether conversations are stored in the cloud.",
    },
]);

maybe_define_setting!(CustomSecretRegexList, group: PrivacySettings, {
    type: Vec<CustomSecretRegex>,
    default: Vec::new(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
    private: false,
    toml_path: "privacy.custom_secret_regex_list",
    description: "Custom regex patterns for detecting and redacting secrets.",
});

maybe_define_setting!(HasInitializedDefaultSecretRegexes, group: PrivacySettings, {
    type: bool,
    default: false,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::No),
    private: true,
});

/// Singleton model for managing the user's privacy settings (whether the user has enabled crash
/// reporting and/or telemetry).
pub struct PrivacySettings {
    auth_state: Arc<AuthState>,
    auth_client: Arc<dyn AuthClient>,
    pub is_telemetry_enabled: bool,
    pub is_crash_reporting_enabled: bool,
    pub is_cloud_conversation_storage_enabled: bool,
    pub has_initialized_default_secret_regexes: HasInitializedDefaultSecretRegexes,
    /// List of user defined secret regexes.
    /// Enterprise-level secret regexes will always take precedence over user-level secrets,
    /// but they both used to support additive behavior.
    /// It's a [Vec<CustomSecretRegex>], but also a user setting.
    pub user_secret_regex_list: CustomSecretRegexList,
    /// List of enterprise-level secret regexes provided by the organization.
    /// These are kept separate from user-level secrets to support additive behavior.
    pub enterprise_secret_regex_list: Vec<CustomSecretRegex>,
    /// Whether or not the user's organization has forced telemetry on, in which case we ignore any
    /// user local/cloud settings. If false, we fall back to the user's settings.
    /// This is populated by the server when teams data is fetched.
    pub is_telemetry_force_enabled: bool,
    /// Whether or not the user's organization has enabled enterprise secret redaction.
    /// This is populated by the server when teams data is fetched.
    pub is_enterprise_secret_redaction_enabled: bool,
}

/// A snapshot of a user's [`PrivacySettings`] settings at some point in time.
#[derive(Clone, Copy)]
pub struct PrivacySettingsSnapshot {
    is_telemetry_enabled: bool,
    is_crash_reporting_enabled: bool,
    is_telemetry_force_enabled: bool,
    should_collect_ai_ugc_telemetry: bool,
    // This is an option so that, if a user has not set this value (and it's set to its default value of true),
    // the default value won't override a value that the user previously set on a different device.
    // This is set to a non-option once the user manually changes this setting.
    cloud_conversation_storage_enabled: Option<bool>,
}

impl PrivacySettingsSnapshot {
    pub fn cloud_conversation_storage_enabled(&self) -> Option<bool> {
        self.cloud_conversation_storage_enabled
    }

    pub fn is_telemetry_enabled(&self) -> bool {
        self.is_telemetry_enabled
    }

    pub fn is_crash_reporting_enabled(&self) -> bool {
        self.is_crash_reporting_enabled
    }

    pub fn is_telemetry_force_enabled(&self) -> bool {
        self.is_telemetry_force_enabled
    }

    pub fn should_disable_telemetry(&self) -> bool {
        // If a user has opted in to the agent mode analytics experiment, telemetry must be enabled.
        !self.is_telemetry_enabled
            && !self.is_telemetry_force_enabled
            && !FeatureFlag::AgentModeAnalytics.is_enabled()
    }

    pub fn should_collect_ai_ugc_telemetry(&self) -> bool {
        self.should_collect_ai_ugc_telemetry
    }

    #[cfg(test)]
    pub fn mock() -> Self {
        Self {
            cloud_conversation_storage_enabled: None,
            is_telemetry_enabled: true,
            is_crash_reporting_enabled: true,
            is_telemetry_force_enabled: true,
            should_collect_ai_ugc_telemetry: true,
        }
    }
}

impl PrivacySettings {
    /// Registers a singleton PrivacySettings model on `app`.
    ///
    /// We expose this function publicly (while keeping the constructor private) to prevent
    /// instantiation another PrivacySettings struct, in the case where a developer might be
    /// unaware that it is registered as a singleton model.
    pub fn register_singleton(ctx: &mut AppContext) {
        let handle = ctx.add_singleton_model(PrivacySettings::new);

        register_settings_events!(
            PrivacySettings,
            user_secret_regex_list,
            CustomSecretRegexList,
            handle,
            ctx
        );
    }

    /// Returns a new PrivacySettings object initialized from locally cached values. Server-side
    /// settings are fetched later via `fetch_or_update_settings`, which is called from
    /// `on_user_fetched` after the user's auth state is established.
    fn new(ctx: &mut ModelContext<Self>) -> Self {
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();

        // Initialize from `WarpDrivePrivacySettings`, which is the source of truth for these
        // booleans.
        let warp_drive_privacy = WarpDrivePrivacySettings::as_ref(ctx);
        let is_telemetry_enabled = *warp_drive_privacy.is_telemetry_enabled.value();
        let is_crash_reporting_enabled = *warp_drive_privacy.is_crash_reporting_enabled.value();
        let is_cloud_conversation_storage_enabled = *warp_drive_privacy
            .is_cloud_conversation_storage_enabled
            .value();

        // Listen for changes to the cloud model and update ourselves when they happen.
        ctx.subscribe_to_model(&WarpDrivePrivacySettings::handle(ctx), |me, event, ctx| {
            let privacy_settings = WarpDrivePrivacySettings::as_ref(ctx);
            match event {
                WarpDrivePrivacySettingsChangedEvent::IsTelemetryEnabled { .. } => {
                    me.set_is_telemetry_enabled(
                        *privacy_settings.is_telemetry_enabled.value(),
                        ctx,
                    );
                }
                WarpDrivePrivacySettingsChangedEvent::IsCrashReportingEnabled { .. } => {
                    me.set_is_crash_reporting_enabled(
                        *privacy_settings.is_crash_reporting_enabled.value(),
                        ctx,
                    );
                }
                WarpDrivePrivacySettingsChangedEvent::IsCloudConversationStorageEnabled {
                    ..
                } => {
                    me.set_is_cloud_conversation_storage_enabled(
                        *privacy_settings
                            .is_cloud_conversation_storage_enabled
                            .value(),
                        ctx,
                    );
                }
            }
        });

        let user_secret_regex_list: CustomSecretRegexList =
            CustomSecretRegexList::new_from_storage(ctx);
        let has_initialized_default_secret_regexes: HasInitializedDefaultSecretRegexes =
            HasInitializedDefaultSecretRegexes::new_from_storage(ctx);

        Self {
            auth_state,
            auth_client,
            is_crash_reporting_enabled,
            is_telemetry_enabled,
            is_cloud_conversation_storage_enabled,
            user_secret_regex_list,
            has_initialized_default_secret_regexes,
            is_telemetry_force_enabled: false,
            is_enterprise_secret_redaction_enabled: false,
            enterprise_secret_regex_list: Vec::new(),
        }
    }

    pub fn is_telemetry_force_enabled(&self) -> bool {
        self.is_telemetry_force_enabled
    }

    pub fn set_is_telemetry_force_enabled(&mut self, is_telemetry_force_enabled: bool) {
        self.is_telemetry_force_enabled = is_telemetry_force_enabled;
    }

    pub fn is_enterprise_secret_redaction_enabled(&self) -> bool {
        self.is_enterprise_secret_redaction_enabled
    }

    pub fn set_enterprise_secret_redaction_settings(
        &mut self,
        enabled: bool,
        enterprise_regexes: Vec<EnterpriseSecretRegex>,
        change_event_reason: ChangeEventReason,
        ctx: &mut ModelContext<Self>,
    ) {
        if enabled {
            // First time: Force enable secret redaction setting (safe mode).
            if !self.is_enterprise_secret_redaction_enabled {
                let safe_mode_settings = SafeModeSettings::handle(ctx);
                ctx.update_model(&safe_mode_settings, |safe_mode_settings, ctx| {
                    let _ = safe_mode_settings.safe_mode_enabled.set_value(true, ctx);
                });
            }

            // Convert EnterpriseSecretRegex to CustomSecretRegex for internal use
            let mut enterprise_secrets = Vec::new();
            for enterprise_regex in enterprise_regexes {
                if let Ok(regex) = Regex::new(&enterprise_regex.pattern) {
                    enterprise_secrets.push(CustomSecretRegex {
                        pattern: regex,
                        name: enterprise_regex.name,
                    });
                } else {
                    log::error!(
                        "Invalid enterprise secret regex pattern: {}",
                        enterprise_regex.pattern
                    );
                }
            }
            self.enterprise_secret_regex_list = enterprise_secrets;
        } else {
            // Clear enterprise secrets when disabled
            self.enterprise_secret_regex_list.clear();
        }

        self.is_enterprise_secret_redaction_enabled = enabled;

        ctx.emit(PrivacySettingsChangedEvent::CustomSecretRegexList {
            change_event_reason,
        });
        ctx.notify();
    }

    pub fn refresh_to_default(&mut self) {
        // TODO(zach): this seems incorrect - should we also update the values on disk?
        self.is_telemetry_enabled = true;
        self.is_crash_reporting_enabled = true;
        self.is_cloud_conversation_storage_enabled = true;
        self.is_telemetry_force_enabled = false;
        self.is_enterprise_secret_redaction_enabled = false;
    }

    /// Fetch the user's privacy settings from the server if any or update the server settings.
    pub fn fetch_or_update_settings(&self, ctx: &mut ModelContext<Self>) {
        let auth_client_clone = self.auth_client.clone();
        let _ = ctx.spawn(
            async move { auth_client_clone.get_user_settings().await },
            Self::initialize_from_fetched_settings_or_update_settings,
        );
    }

    /// Initializes state from the [`SyncedUserSettings`] fetched from the server, if any.
    /// If there are no settings from the server, updates the server settings with local settings.
    /// TODO: Make this a server-side db transaction.
    fn initialize_from_fetched_settings_or_update_settings(
        &mut self,
        fetched_settings: Result<Option<SyncedUserSettings>>,
        ctx: &mut ModelContext<PrivacySettings>,
    ) {
        match fetched_settings {
            Ok(Some(fetched_settings)) => {
                // Until the login experience stops hiding the telemetry settings,
                // we assume that locally enabled telemetry is unintentional.
                // As such, where settings differ, we respect whichever setting that is disabled.
                self.overwrite_local_settings_if_cloud_disabled(fetched_settings, ctx);
                // If any local setting is disabled, we have to update the server.
                if !self.is_telemetry_enabled
                    || !self.is_crash_reporting_enabled
                    || !self.is_cloud_conversation_storage_enabled
                {
                    self.update_server_with_local_settings(ctx);
                }
            }
            Ok(None) => {
                // This indicates the user had not logged in before.
                log::info!("User has no synced privacy settings.");
                self.update_server_with_local_settings(ctx);
            }
            Err(err) => {
                report_error!(err.context("Failed to fetch user settings."));
            }
        }

        self.maybe_sync_with_warp_drive_prefs(ctx);
    }

    fn overwrite_local_settings_if_cloud_disabled(
        &mut self,
        fetched_settings: SyncedUserSettings,
        ctx: &mut ModelContext<Self>,
    ) {
        // For now, only overwrite the user's locally stored setting if the cloud version
        // has is_crash_reporting disabled. Until we implement a more reliable retry
        // mechanism for update settings requests, in addition to possibly a UI for the
        // user to resolve the conflicting settings themselves, default to "safe" behavior.
        // Namely, we want to avoid incidentally overwriting is_crash_reporting_enabled to
        // `true`.
        if self.is_crash_reporting_enabled && !fetched_settings.is_crash_reporting_enabled {
            self.set_is_crash_reporting_enabled(fetched_settings.is_crash_reporting_enabled, ctx);
        }

        // For now, only overwrite the user's locally stored setting if the cloud version
        // has is_telemetry_enabled disabled. Until we implement a more reliable retry
        // mechanism for update settings requests, in addition to possibly a UI for the
        // user to resolve the conflicting settings themselves, default to "safe" behavior.
        // Namely, we want to avoid incidentally overwriting is_telemetry_enabled to
        // `true`.
        if self.is_telemetry_enabled && !fetched_settings.is_telemetry_enabled {
            self.set_is_telemetry_enabled(fetched_settings.is_telemetry_enabled, ctx);
        }

        if self.is_cloud_conversation_storage_enabled
            && !fetched_settings.is_cloud_conversation_storage_enabled
        {
            self.set_is_cloud_conversation_storage_enabled(
                fetched_settings.is_cloud_conversation_storage_enabled,
                ctx,
            );
        }
    }

    /// Constructor for tests only.
    #[cfg(test)]
    pub fn mock(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            auth_state: Arc::new(AuthState::new_for_test()),
            auth_client: Arc::new(MockAuthClient::new()),
            is_crash_reporting_enabled: true,
            is_telemetry_enabled: true,
            is_cloud_conversation_storage_enabled: true,
            user_secret_regex_list: CustomSecretRegexList::new(None),
            has_initialized_default_secret_regexes: HasInitializedDefaultSecretRegexes::new(None),
            is_telemetry_force_enabled: false,
            is_enterprise_secret_redaction_enabled: false,
            enterprise_secret_regex_list: Vec::new(),
        }
    }

    /// Returns a snapshot of the user's privacy settings.
    ///
    /// The returned snapshot is not stateful, thus its values should be used shortly after the
    /// snapshot is returned.
    pub fn get_snapshot(&self, app: &AppContext) -> PrivacySettingsSnapshot {
        PrivacySettingsSnapshot {
            cloud_conversation_storage_enabled: (!self.is_cloud_conversation_storage_enabled)
                .then_some(false),
            is_telemetry_enabled: self.is_telemetry_enabled,
            is_crash_reporting_enabled: self.is_crash_reporting_enabled,
            is_telemetry_force_enabled: self.is_telemetry_force_enabled,
            should_collect_ai_ugc_telemetry: should_collect_ai_ugc_telemetry(
                app,
                self.is_telemetry_enabled,
            ),
        }
    }

    /// Sets `is_crash_reporting_enabled` to the given value.
    ///
    /// Additionally, this writes the given value to the user's local defaults, and additionally
    /// sends a request to update the user's `is_crash_reporting_enabled` value stored server-side.
    /// Finally, emits a `PrivacySettingsEvent::UpdateIsCrashReportingEnabled` event.
    pub fn set_is_crash_reporting_enabled(
        &mut self,
        new_value: bool,
        ctx: &mut ModelContext<PrivacySettings>,
    ) {
        let old_value = self.is_crash_reporting_enabled;
        if new_value != old_value {
            self.is_crash_reporting_enabled = new_value;

            WarpDrivePrivacySettings::handle(ctx).update(ctx, |settings, ctx| {
                log::info!("Setting is_crash_reporting_enabled to {new_value}");
                let _ = settings
                    .is_crash_reporting_enabled
                    .set_value(new_value, ctx);
            });

            if self.auth_state.is_logged_in() {
                let auth_client = self.auth_client.clone();
                let _ = ctx.spawn(
                    async move { auth_client.set_is_crash_reporting_enabled(new_value).await },
                    |_, _, _| (),
                );
            }
            ctx.emit(PrivacySettingsChangedEvent::UpdateIsCrashReportingEnabled {
                old_value,
                new_value,
            });
            ctx.notify();
        }
    }

    /// Sets `is_telemetry_enabled` to the given value.
    ///
    /// Additionally, this writes the given value to the user's local defaults, and additionally
    /// sends a request to update the user's `is_telemetry_enabled` value stored server-side.
    /// Finally, emits a `PrivacySettingsEvent::UpdateIsTelemetryEnabled` event.
    pub fn set_is_telemetry_enabled(
        &mut self,
        new_value: bool,
        ctx: &mut ModelContext<PrivacySettings>,
    ) {
        let old_value = self.is_telemetry_enabled;
        if new_value != old_value {
            self.is_telemetry_enabled = new_value;

            WarpDrivePrivacySettings::handle(ctx).update(ctx, |settings, ctx| {
                log::info!("Setting is_telemetry_enabled to {new_value}");
                let _ = settings.is_telemetry_enabled.set_value(new_value, ctx);
            });

            if self.auth_state.is_logged_in() {
                let auth_client = self.auth_client.clone();
                let _ = ctx.spawn(
                    async move { auth_client.set_is_telemetry_enabled(new_value).await },
                    |_, _, _| (),
                );
            }
            ctx.emit(PrivacySettingsChangedEvent::UpdateIsTelemetryEnabled {
                old_value,
                new_value,
            });
            ctx.notify();
        }
    }

    pub fn set_is_cloud_conversation_storage_enabled(
        &mut self,
        new_value: bool,
        ctx: &mut ModelContext<PrivacySettings>,
    ) {
        let old_value = self.is_cloud_conversation_storage_enabled;
        if new_value == old_value {
            return;
        }

        self.is_cloud_conversation_storage_enabled = new_value;

        WarpDrivePrivacySettings::handle(ctx).update(ctx, |settings, ctx| {
            log::info!("Setting is_cloud_conversation_storage_enabled to {new_value}");
            let _ = settings
                .is_cloud_conversation_storage_enabled
                .set_value(new_value, ctx);
        });

        if self.auth_state.is_logged_in() {
            let auth_client = self.auth_client.clone();
            let _ = ctx.spawn(
                async move {
                    auth_client
                        .set_is_cloud_conversation_storage_enabled(new_value)
                        .await
                },
                |_, _, _| (),
            );
        }

        ctx.emit(
            PrivacySettingsChangedEvent::UpdateIsCloudConversationStorageEnabled {
                old_value,
                new_value,
            },
        );
        ctx.notify();
    }

    pub fn remove_user_secret_regex(&mut self, idx: &usize, ctx: &mut ModelContext<Self>) {
        let mut new_user_secret_regex_list = self.user_secret_regex_list.to_vec();
        new_user_secret_regex_list.remove(*idx);
        if self
            .user_secret_regex_list
            .set_value(new_user_secret_regex_list, ctx)
            .is_err()
        {
            log::error!("Custom Secret Regex List failed to serialize")
        }
    }

    /// Initializes the custom secret regex list with the default regexes, provided
    /// non matches can be found.
    /// This can be called when a user first enables secret redaction.
    pub fn add_all_recommended_regex(&mut self, ctx: &mut ModelContext<Self>) {
        let mut new_user_secret_regex_list = self.user_secret_regex_list.to_vec();
        let num_existing_regexes = new_user_secret_regex_list.len();

        // Add all the default regexes if they don't already exist
        for default_regex in crate::terminal::model::secrets::regexes::DEFAULT_REGEXES_WITH_NAMES {
            if let Ok(regex) = Regex::new(default_regex.pattern) {
                let custom_regex = CustomSecretRegex {
                    pattern: regex,
                    name: Some(default_regex.name.to_string()),
                };
                if !new_user_secret_regex_list.contains(&custom_regex) {
                    new_user_secret_regex_list.push(custom_regex);
                }
            } else {
                log::error!("Failed to compile default regex: {}", default_regex.pattern);
            }
        }

        if num_existing_regexes == new_user_secret_regex_list.len() {
            return;
        }

        if self
            .user_secret_regex_list
            .set_value(new_user_secret_regex_list, ctx)
            .is_err()
        {
            log::error!("Failed to serialize default regexes to custom secret regex list")
        }

        ctx.notify();
    }

    /// Disables the default regex trigger, so that it will not be executed.
    pub fn disable_default_regex_trigger(&mut self, ctx: &mut ModelContext<Self>) {
        if self
            .has_initialized_default_secret_regexes
            .set_value(true, ctx)
            .is_err()
        {
            log::error!("Failed to disable default regex trigger");
        }
    }

    /// Initializes the custom secret regex list with the default regexes.
    /// This will only be executed once per user, and only if they haven't already initialized.
    pub fn initialize_default_regexes_once(&mut self, ctx: &mut ModelContext<Self>) {
        // Only initialize if we haven't done so before
        if !*self.has_initialized_default_secret_regexes.value() {
            self.add_all_recommended_regex(ctx);

            // Mark as initialized
            if self
                .has_initialized_default_secret_regexes
                .set_value(true, ctx)
                .is_err()
            {
                log::error!("Failed to set has_initialized_default_secret_regexes flag");
            }
        }
    }

    /// Sends request(s) to update server-side user settings with current local values.
    fn update_server_with_local_settings(&self, ctx: &mut ModelContext<Self>) {
        if self.auth_state.is_logged_in() {
            let auth_client = self.auth_client.clone();
            let snapshot = self.get_snapshot(ctx);
            let _ = ctx.spawn(
                async move {
                    let result = auth_client.update_user_settings(snapshot).await;
                    if let Err(err) = result {
                        report_error!(
                            err.context("Failed to update server with local privacy settings.")
                        )
                    }
                },
                |_, _, _| (),
            );
        }
    }

    /// We wait until warp drive prefs have loaded and then either
    /// 1) use them as the data store for is_telemetry_enabled and is_crash_reporting_enabled, if those
    ///    values are set in warp drive, or
    /// 2) update the warp drive prefs to match the values from the legacy user_settings endpoint so
    ///    that we can use warp drive prefs going forward.
    pub fn maybe_sync_with_warp_drive_prefs(&mut self, ctx: &mut ModelContext<Self>) {
        // Wait for cloud objects to load, and, if telemetry & crash reporting are synced to warp drive
        // initialize from the warp drive values.
        let update_manager = UpdateManager::as_ref(ctx);
        ctx.spawn(
            update_manager.initial_load_complete(),
            Self::handle_warp_drive_objects_loaded,
        );
    }

    fn handle_warp_drive_objects_loaded(&mut self, _: (), ctx: &mut ModelContext<Self>) {
        self.initialize_default_regexes_once(ctx);
        // Check if the warp drive preferences are set. If they are, and telemetry and crash reporting
        // are set as warp drive prefs, then use those.  Otherwise, update the warp drive prefs to match
        // the values from the legacy user_settings endpoint so that we can use warp drive prefs going forward.
        let cloud_model = CloudModel::as_ref(ctx);
        let cloud_prefs = cloud_model.get_all_cloud_preferences_by_storage_key();
        let cloud_telemetry_value =
            cloud_prefs
                .get(IsTelemetryEnabled::storage_key())
                .map(|pref| {
                    pref.model()
                        .string_model
                        .value
                        .as_bool()
                        .unwrap_or_default()
                });
        let cloud_crash_reporting_value = cloud_prefs
            .get(IsCrashReportingEnabled::storage_key())
            .map(|pref| {
                pref.model()
                    .string_model
                    .value
                    .as_bool()
                    .unwrap_or_default()
            });
        let cloud_conversation_storage_value = cloud_prefs
            .get(IsCloudConversationStorageEnabled::storage_key())
            .map(|pref| {
                pref.model()
                    .string_model
                    .value
                    .as_bool()
                    .unwrap_or_default()
            });

        match (
            cloud_telemetry_value,
            cloud_crash_reporting_value,
            cloud_conversation_storage_value,
        ) {
            (
                Some(is_telemetry_enabled),
                Some(is_crash_reporting_enabled),
                Some(is_cloud_conversation_storage_enabled),
            ) => {
                log::info!(
                    "Warp Drive privacy preferences are set, using those for telemetry={is_telemetry_enabled}, \
                    crash_reporting={is_crash_reporting_enabled}, cloud_conversation_storage={is_cloud_conversation_storage_enabled}"
                );
                self.set_is_telemetry_enabled(is_telemetry_enabled, ctx);
                self.set_is_crash_reporting_enabled(is_crash_reporting_enabled, ctx);
                self.set_is_cloud_conversation_storage_enabled(
                    is_cloud_conversation_storage_enabled,
                    ctx,
                );
            }
            _ => {
                log::info!(
                    "Warp Drive privacy preferences are not set, syncing local PrivacySettings values to \
                    WarpDrivePrivacySettings and cloud. telemetry={}, crash_reporting={}, \
                    cloud_conversation_storage={}",
                    self.is_telemetry_enabled,
                    self.is_crash_reporting_enabled,
                    self.is_cloud_conversation_storage_enabled
                );
                // First, ensure WarpDrivePrivacySettings (the define_settings_group model)
                // reflects the actual PrivacySettings in-memory values. These may differ
                // because WarpDrivePrivacySettings defaults to `true` for all three settings,
                // while the user may have changed them to `false` via PrivacySettings before
                // signing up. Without this step, maybe_sync_local_prefs_to_cloud would read
                // the stale WarpDrivePrivacySettings defaults and push those to the cloud.
                WarpDrivePrivacySettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .is_telemetry_enabled
                        .set_value(self.is_telemetry_enabled, ctx));
                    report_if_error!(settings
                        .is_crash_reporting_enabled
                        .set_value(self.is_crash_reporting_enabled, ctx));
                    report_if_error!(settings
                        .is_cloud_conversation_storage_enabled
                        .set_value(self.is_cloud_conversation_storage_enabled, ctx));
                });
                CloudPreferencesSyncer::handle(ctx).update(ctx, |syncer, ctx| {
                    syncer.maybe_sync_local_prefs_to_cloud(
                        vec![
                            IsTelemetryEnabled::storage_key().to_string(),
                            IsCrashReportingEnabled::storage_key().to_string(),
                            IsCloudConversationStorageEnabled::storage_key().to_string(),
                        ],
                        ctx,
                    );
                });
            }
        }
    }
}

/// Events emitted when PrivacySettings is updated.
#[derive(Clone, Copy)]
pub enum PrivacySettingsChangedEvent {
    UpdateIsTelemetryEnabled {
        old_value: bool,
        new_value: bool,
    },
    UpdateIsCrashReportingEnabled {
        old_value: bool,
        new_value: bool,
    },
    UpdateIsCloudConversationStorageEnabled {
        old_value: bool,
        new_value: bool,
    },
    CustomSecretRegexList {
        change_event_reason: ChangeEventReason,
    },
    HasInitializedDefaultSecretRegexes {
        change_event_reason: ChangeEventReason,
    },
}

impl Entity for PrivacySettings {
    type Event = PrivacySettingsChangedEvent;
}

impl SingletonEntity for PrivacySettings {}
