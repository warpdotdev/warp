use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context;
use tempfile::{Builder, NamedTempFile};
use vec1::Vec1;
use warp_core::safe_info;
use warp_managed_secrets::ManagedSecretManager;
use warpui::{ModelSpawner, SingletonEntity};

use super::super::terminal::TerminalDriver;
use super::{CloudProvider, CloudProviderSetupError, Result};
use crate::ai::aws_credentials::aws_role_session_name;
use crate::ai::cloud_environments::AwsProviderConfig;

/// Default duration for OIDC identity tokens issued for cloud provider auth.
/// The AWS CLI doesn't offer a mechanism for refreshing web identity tokens, so we
/// set this to the current maximum task duration.
const IDENTITY_TOKEN_DURATION: Duration = Duration::from_hours(3);

/// AWS STS audience for Warp Oz OIDC federation.
const AWS_AUDIENCE: &str = "sts.amazonaws.com";

/// Provides AWS Web Identity credentials for the agent session.
pub(crate) struct AwsCloudProvider {
    /// ARN of the role to assume.
    role_arn: String,
    session_name: String,
    /// File containing the OIDC token that the AWS CLI will use to assume the role.
    token_file: NamedTempFile,
}

impl AwsCloudProvider {
    const PROVIDER_NAME: &'static str = "aws";

    pub fn new(config: &AwsProviderConfig, run_id: &str) -> Result<Self> {
        // The `tempfile` crate defaults to creating temporary files with user-only permissions.
        let token_file = Builder::new()
            .prefix(&format!("oz_aws_oidc_{run_id}_"))
            .suffix(".token")
            .tempfile()
            .context("Failed to create temporary AWS OIDC token file")
            .map_err(|error| CloudProviderSetupError::new(Self::PROVIDER_NAME, error))?;

        Ok(Self {
            role_arn: config.role_arn.clone(),
            session_name: aws_role_session_name(run_id),
            token_file,
        })
    }
}

impl CloudProvider for AwsCloudProvider {
    fn env_vars(&self) -> Result<HashMap<OsString, OsString>> {
        // Set variables that the AWS CLI and SDKs check for assuming a role with web identity:
        // https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-role.html#cli-configure-role-oidc
        let mut vars = HashMap::new();
        vars.insert(
            OsString::from("AWS_ROLE_ARN"),
            OsString::from(&self.role_arn),
        );
        vars.insert(
            OsString::from("AWS_ROLE_SESSION_NAME"),
            OsString::from(&self.session_name),
        );
        vars.insert(
            OsString::from("AWS_WEB_IDENTITY_TOKEN_FILE"),
            self.token_file.path().as_os_str().to_owned(),
        );
        Ok(vars)
    }

    fn setup(
        &mut self,
        spawner: ModelSpawner<TerminalDriver>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            let token_file_path = self.token_file.path();
            safe_info!(
                safe: ("Setting up AWS cloud provider credentials"),
                full: ("Setting up AWS cloud provider: role_arn={}, session={}", self.role_arn, self.session_name)
            );

            // 1. Issue an OIDC identity token.
            let audience = AWS_AUDIENCE.to_string();
            let duration = IDENTITY_TOKEN_DURATION;

            // Use the scoped principal as the subject, since AWS can't match directly
            // on the team claim.
            let subject_template = Vec1::new("scoped_principal".into());
            let token = spawner
                .spawn(move |_, ctx| {
                    ManagedSecretManager::handle(ctx)
                        .as_ref(ctx)
                        .issue_task_identity_token(
                            warp_managed_secrets::client::IdentityTokenOptions {
                                audience,
                                requested_duration: duration,
                                subject_template,
                            },
                        )
                })
                .await
                .map_err(|err| CloudProviderSetupError::new(Self::PROVIDER_NAME, err))?
                .await
                .map_err(|err| CloudProviderSetupError::new(Self::PROVIDER_NAME, err))?;

            // 2. Write the token to the pre-created temporary file.
            async_fs::write(&token_file_path, token.token.as_bytes())
                .await
                .map_err(|err| CloudProviderSetupError::new(Self::PROVIDER_NAME, err))?;

            safe_info!(
                safe: ("AWS cloud provider setup complete"),
                full: ("AWS cloud provider setup complete: token_file={}", token_file_path.display())
            );
            Ok(())
        })
    }

    fn cleanup(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        Box::pin(async move {
            let Self { token_file, .. } = *self;
            token_file
                .close()
                .context("Failed to remove AWS OIDC token file")
                .map_err(|err| CloudProviderSetupError::new(Self::PROVIDER_NAME, err))
        })
    }
}
