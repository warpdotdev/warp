use warp_graphql::mutations::update_user_settings::UpdateUserSettingsResult;

use super::AuthClientImpl;

#[test]
fn unknown_settings_results_preserve_operation_context() {
    for expected_message in [
        "failed to set telemetry enabled",
        "failed to set crash reporting enabled",
        "failed to set cloud conversation storage enabled",
        "failed to update user settings",
    ] {
        let error = AuthClientImpl::on_settings_updated(
            UpdateUserSettingsResult::Unknown,
            expected_message,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), expected_message);
    }
}
