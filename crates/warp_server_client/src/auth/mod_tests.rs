use cynic::{GraphQlError, GraphQlErrorPathSegment, GraphQlResponse};
use warp_graphql::mutations::update_user_settings::UpdateUserSettingsResult;
use warp_graphql::queries::get_user::{GetUser, UserResult};

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

#[test]
fn missing_user_data_surfaces_graphql_response_errors() {
    let response = GraphQlResponse::<GetUser> {
        data: None,
        errors: Some(vec![
            GraphQlError::new(
                "failed to mint ID token: token is expired".to_string(),
                None,
                Some(vec![GraphQlErrorPathSegment::Field("user".to_string())]),
                None,
            ),
            GraphQlError::new("user not in context".to_string(), None, None, None),
        ]),
    };

    let error = AuthClientImpl::user_output_from_response(Some("GetUser"), response).unwrap_err();

    let message = error.to_string();
    assert!(
        message.contains("failed to mint ID token: token is expired"),
        "expected the GraphQL error message to be surfaced, got {message}"
    );
    assert!(
        message.contains("at user"),
        "expected the GraphQL error path to be surfaced, got {message}"
    );
    assert!(
        message.contains("user not in context"),
        "expected every GraphQL error message to be surfaced, got {message}"
    );
    assert!(
        message.contains("GetUser"),
        "expected the operation name to be surfaced, got {message}"
    );
}

#[test]
fn missing_user_data_without_response_errors_falls_back_to_generic_message() {
    let response = GraphQlResponse::<GetUser> {
        data: None,
        errors: None,
    };

    let error = AuthClientImpl::user_output_from_response(Some("GetUser"), response).unwrap_err();

    assert_eq!(error.to_string(), "missing response data for GetUser");
}

#[test]
fn unknown_user_result_preserves_existing_error() {
    let response = GraphQlResponse {
        data: Some(GetUser {
            user: UserResult::Unknown,
        }),
        errors: None,
    };

    let error = AuthClientImpl::user_output_from_response(Some("GetUser"), response).unwrap_err();

    assert_eq!(error.to_string(), "Unable to fetch user");
}
