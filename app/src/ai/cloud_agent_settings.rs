//! Settings for cloud agent functionality.
//!
//! This module contains user-specific settings for cloud agent features,
//! such as remembering the last selected environment.

use std::collections::HashMap;

use settings::macros::define_settings_group;
use settings::{Setting as _, SupportedPlatforms, SyncToCloud};
use warp_cli::agent::Harness;
use warp_errors::report_if_error;

use crate::server::ids::SyncId;
use crate::workspaces::user_workspaces::TeamScope;

#[derive(
    Clone,
    Debug,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "Selected third-party harness model.")]
pub struct HarnessModelSelection {
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_level: Option<String>,
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
pub enum AuthSecretPreference {
    Named(String),
    Inherit,
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
struct AuthSecretPreferenceScope {
    team_uid: Option<String>,
}

impl AuthSecretPreferenceScope {
    fn from_scope(scope: &(impl TeamScope + ?Sized)) -> Self {
        Self {
            team_uid: scope.team_uid().map(|team_uid| team_uid.uid()),
        }
    }
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
pub struct ScopedAuthSecretPreference {
    team_scope: AuthSecretPreferenceScope,
    harness: String,
    preference: AuthSecretPreference,
}

define_settings_group!(CloudAgentSettings, settings: [
    last_selected_environment_id: LastSelectedEnvironmentId {
        type: Option<SyncId>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    harness_auth_ftux_completed: HarnessAuthFtuxCompleted {
        type: HashMap<String, bool>,
        default: HashMap::new(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    last_selected_harness: LastSelectedHarness {
        type: Option<String>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    last_selected_host: LastSelectedHost {
        type: Option<String>,
        default: None,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    last_selected_harness_model: LastSelectedHarnessModel {
        type: HashMap<String, HarnessModelSelection>,
        default: HashMap::new(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    // Legacy migration fallback. New code must use `auth_secret_preference` and
    // `persist_auth_secret_preference`.
    last_selected_auth_secret: LastSelectedAuthSecret {
        type: HashMap<String, String>,
        default: HashMap::new(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    // Legacy migration fallback for explicit Inherit choices. New code must use
    // `auth_secret_preference` and `persist_auth_secret_preference`.
    inherit_auth_secret_harnesses: InheritAuthSecretHarnesses {
        type: HashMap<String, bool>,
        default: HashMap::new(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    },
    scoped_auth_secret_preferences: ScopedAuthSecretPreferences {
        type: Vec<ScopedAuthSecretPreference>,
        default: Vec::new(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        surface: settings::SettingSurfaces::GUI,
        private: true,
    }
]);

impl CloudAgentSettings {
    pub fn auth_secret_preference<S: TeamScope + ?Sized>(
        &self,
        team_scope: &S,
        harness: Harness,
    ) -> Option<AuthSecretPreference> {
        let persisted_scope = AuthSecretPreferenceScope::from_scope(team_scope);
        self.scoped_auth_secret_preferences
            .value()
            .iter()
            .find(|entry| {
                entry.team_scope == persisted_scope && entry.harness == harness.config_name()
            })
            .map(|entry| entry.preference.clone())
            .or_else(|| {
                if self
                    .inherit_auth_secret_harnesses
                    .value()
                    .get(harness.config_name())
                    .copied()
                    .unwrap_or(false)
                {
                    Some(AuthSecretPreference::Inherit)
                } else {
                    self.last_selected_auth_secret
                        .value()
                        .get(harness.config_name())
                        .cloned()
                        .map(AuthSecretPreference::Named)
                }
            })
    }

    pub fn persist_auth_secret_preference<S: TeamScope + ?Sized>(
        &mut self,
        team_scope: &S,
        harness: Harness,
        preference: Option<AuthSecretPreference>,
        ctx: &mut warpui::ModelContext<Self>,
    ) {
        let persisted_scope = AuthSecretPreferenceScope::from_scope(team_scope);
        let harness_key = harness.config_name().to_string();
        let mut preferences = self.scoped_auth_secret_preferences.value().clone();
        preferences.retain(|entry| {
            entry.team_scope != persisted_scope || entry.harness.as_str() != harness_key
        });
        if let Some(preference) = preference {
            preferences.push(ScopedAuthSecretPreference {
                team_scope: persisted_scope,
                harness: harness_key,
                preference,
            });
        }
        report_if_error!(
            self.scoped_auth_secret_preferences
                .set_value(preferences, ctx)
        );
    }
    pub fn is_harness_auth_ftux_completed(&self, harness: Harness) -> bool {
        self.harness_auth_ftux_completed
            .value()
            .get(harness.config_name())
            .copied()
            .unwrap_or(false)
    }

    pub fn mark_harness_auth_ftux_completed(
        &mut self,
        harness: Harness,
        ctx: &mut warpui::ModelContext<Self>,
    ) {
        let mut map = self.harness_auth_ftux_completed.value().clone();
        map.insert(harness.config_name().to_string(), true);
        let _ = self.harness_auth_ftux_completed.set_value(map, ctx);
    }

    /// Persists (or clears) the harness model selection for the given harness.
    pub fn persist_harness_model_selection(
        &mut self,
        harness: Harness,
        model_id: &str,
        reasoning_level: Option<String>,
        ctx: &mut warpui::ModelContext<Self>,
    ) {
        let mut map = self.last_selected_harness_model.value().clone();
        if model_id.is_empty() {
            map.remove(harness.config_name());
        } else {
            map.insert(
                harness.config_name().to_string(),
                HarnessModelSelection {
                    model_id: model_id.to_string(),
                    reasoning_level,
                },
            );
        }
        report_if_error!(self.last_selected_harness_model.set_value(map, ctx));
    }
}

#[cfg(test)]
#[path = "cloud_agent_settings_tests.rs"]
mod tests;
