use regex::Regex;
use warp_errors::report_error;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::server::telemetry::secret_redaction::update_telemetry_secrets_regex;
use crate::settings::{CustomSecretRegex, PrivacySettings, PrivacySettingsChangedEvent};
use crate::terminal::model::set_user_and_enterprise_secret_regexes;
use crate::workspaces::user_workspaces::{UserWorkspaces, UserWorkspacesEvent};

/// Dummy singleton model that is used to update the current set of custom regexes within the
/// terminal model. We do this via a singleton model since we only want to do this once any time
/// the custom secret regex list changes, which must be done independent of any view.
pub struct CustomSecretRegexUpdater;

impl CustomSecretRegexUpdater {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let updater = CustomSecretRegexUpdater;
        // Initialize with current custom regexes (will be empty until safe mode is enabled)
        updater.update_custom_secret_regex_list(ctx);

        let privacy_settings = PrivacySettings::handle(ctx);
        ctx.subscribe_to_model(&privacy_settings, |me, _, evt, ctx| {
            if let PrivacySettingsChangedEvent::CustomSecretRegexList { .. } = evt {
                me.update_custom_secret_regex_list(ctx);
            }
        });

        // The enterprise half of the regex list lives on `UserWorkspaces`, not
        // `PrivacySettings` -- see the doc comment on
        // `PrivacySettings::is_enterprise_secret_redaction_enabled` -- so it must be
        // recomputed whenever team data changes rather than via a `PrivacySettings` event.
        let user_workspaces = UserWorkspaces::handle(ctx);
        ctx.subscribe_to_model(&user_workspaces, |me, _, evt, ctx| {
            if let UserWorkspacesEvent::TeamsChanged = evt {
                me.update_custom_secret_regex_list(ctx);
            }
        });

        updater
    }

    fn update_custom_secret_regex_list(&self, ctx: &mut ModelContext<Self>) {
        let user_secrets: Vec<Regex> = PrivacySettings::as_ref(ctx)
            .user_secret_regex_list
            .iter()
            .map(CustomSecretRegex::pattern)
            .cloned()
            .collect();

        let enterprise_secrets: Vec<Regex> = UserWorkspaces::as_ref(ctx)
            .get_enterprise_secret_redaction_regex_list()
            .into_iter()
            .filter_map(
                |enterprise_regex| match Regex::new(&enterprise_regex.pattern) {
                    Ok(regex) => Some(regex),
                    Err(_) => {
                        report_error!(
                            "Invalid enterprise secret regex pattern",
                            extra: { "pattern" => %enterprise_regex.pattern }
                        );
                        None
                    }
                },
            )
            .collect();

        set_user_and_enterprise_secret_regexes(user_secrets.iter(), enterprise_secrets.iter());

        // Also update the telemetry-side secret regex, which is independent of
        // the user's safe-mode setting and always includes the default patterns.
        update_telemetry_secrets_regex(user_secrets.iter(), enterprise_secrets.iter());
    }
}

impl Entity for CustomSecretRegexUpdater {
    type Event = ();
}

impl SingletonEntity for CustomSecretRegexUpdater {}
