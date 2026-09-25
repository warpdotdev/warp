use std::time::Duration;

use aws_credential_types::provider::error::CredentialsError;

use super::{bedrock_identity_token_error, user_facing_aws_credentials_error_message};
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::server::server_api::managed_secrets::IdentityTokenUserFacingError;

#[test]
fn bedrock_mint_error_preserves_safe_server_message_in_run_failure() {
    let message = "Cannot issue a team-scoped identity token: you belong to multiple teams. Select a team using the X-Warp-Team-Uid request header.";
    let error = anyhow::Error::new(IdentityTokenUserFacingError(message.to_string()));
    let error = bedrock_identity_token_error(error);
    let run_error = AgentDriverError::AwsBedrockCredentialsFailed(error.to_string());
    assert_eq!(
        run_error.to_string(),
        format!(
            "Failed to initialize AWS Bedrock credentials: Failed to mint AWS Bedrock task identity token: {message}"
        )
    );
}

#[test]
fn bedrock_mint_error_does_not_surface_untrusted_error_details() {
    let error = anyhow::anyhow!("untrusted response with sensitive material")
        .context("HTTP request failed");
    assert_eq!(
        bedrock_identity_token_error(error).to_string(),
        "Failed to mint AWS Bedrock task identity token"
    );
}

#[test]
fn maps_credentials_not_loaded_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::not_loaded_no_source(),
        "sandbox",
    );

    assert_eq!(
        message,
        "AWS credentials were not found for the AWS profile `sandbox`. Log in with the AWS CLI or update your AWS credentials configuration, then refresh."
    );
}

#[test]
fn maps_invalid_configuration_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::invalid_configuration(std::io::Error::other("bad config")),
        "readonly",
    );

    assert_eq!(
        message,
        "The AWS profile `readonly` is invalid or incomplete in your local AWS configuration. Update your AWS profile settings and credentials, then refresh."
    );
}

#[test]
fn maps_provider_timeout_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::provider_timed_out(Duration::from_secs(5)),
        "sandbox",
    );

    assert_eq!(
        message,
        "Timed out while loading AWS credentials. Refresh and try again."
    );
}

#[test]
fn maps_provider_error_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::provider_error(std::io::Error::other("provider error")),
        "sandbox",
    );

    assert_eq!(
        message,
        "Unable to load AWS credentials from your configured provider. Refresh your AWS login and try again."
    );
}

#[test]
fn maps_unhandled_error_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::unhandled(std::io::Error::other("unexpected")),
        "sandbox",
    );

    assert_eq!(
        message,
        "Unexpected error while loading AWS credentials. Refresh your AWS login and try again."
    );
}
