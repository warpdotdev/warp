use warpui::{Entity, ModelContext, SingletonEntity};

use crate::server::telemetry::secret_redaction::update_telemetry_secrets_regex;
use crate::settings::{CustomSecretRegex, PrivacySettings, PrivacySettingsChangedEvent};
use crate::terminal::model::set_user_and_enterprise_secret_regexes;
use crate::terminal::safe_mode_settings::{SafeModeSettings, SafeModeSettingsChangedEvent};

/// Dummy singleton model that is used to update the current set of custom regexes within the
/// terminal model. We do this via a singleton model since we only want to do this once any time
/// the custom secret regex list changes, which must be done independent of any view.
pub struct CustomSecretRegexUpdater;

impl CustomSecretRegexUpdater {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let updater = CustomSecretRegexUpdater;
        // Push the current custom regexes into the global secret-scanning DFA.
        // The list may already be populated from TOML at app startup, in which
        // case this is the only chance to load it — subsequent recompiles
        // happen via the subscriptions below.
        updater.update_custom_secret_regex_list(ctx);

        let privacy_settings = PrivacySettings::handle(ctx);
        ctx.subscribe_to_model(&privacy_settings, |me, _, evt, ctx| {
            if let PrivacySettingsChangedEvent::CustomSecretRegexList { .. } = evt {
                me.update_custom_secret_regex_list(ctx);
            }
        });

        // Also recompile when the user flips Safe Mode at runtime. Toggling
        // `safe_mode_enabled` doesn't mutate the regex list — it only changes
        // the boolean that gates redaction — so the `CustomSecretRegexList`
        // subscription above doesn't fire. Without this second subscription,
        // a user whose regex list was loaded from TOML can flip Safe Mode ON
        // and still have an empty in-memory DFA, which silently bypasses the
        // local MCP-save redaction check that #10839 wired up. See #11262.
        let safe_mode_settings = SafeModeSettings::handle(ctx);
        ctx.subscribe_to_model(&safe_mode_settings, |me, evt, ctx| {
            if let SafeModeSettingsChangedEvent::SafeModeEnabled { .. } = evt {
                me.update_custom_secret_regex_list(ctx);
            }
        });

        updater
    }

    fn update_custom_secret_regex_list(&self, ctx: &mut ModelContext<Self>) {
        let privacy_settings = PrivacySettings::as_ref(ctx);

        // Get enterprise and user secrets separately
        let enterprise_secrets = privacy_settings
            .enterprise_secret_regex_list
            .iter()
            .map(CustomSecretRegex::pattern);

        let user_secrets = privacy_settings
            .user_secret_regex_list
            .iter()
            .map(CustomSecretRegex::pattern);

        set_user_and_enterprise_secret_regexes(user_secrets, enterprise_secrets);

        // Also update the telemetry-side secret regex, which is independent of
        // the user's safe-mode setting and always includes the default patterns.
        let enterprise_secrets = privacy_settings
            .enterprise_secret_regex_list
            .iter()
            .map(CustomSecretRegex::pattern);

        let user_secrets = privacy_settings
            .user_secret_regex_list
            .iter()
            .map(CustomSecretRegex::pattern);

        update_telemetry_secrets_regex(user_secrets, enterprise_secrets);
    }
}

impl Entity for CustomSecretRegexUpdater {
    type Event = ();
}

impl SingletonEntity for CustomSecretRegexUpdater {}
