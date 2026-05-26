use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context as _;
use warp_managed_secrets::{GcpCredentials, GcpFederationConfig};

use super::{CloudProvider, CloudProviderSetupError, Result};
use crate::ai::cloud_environments::GcpProviderConfig;

/// Token lifetime for GCP executable-sourced credentials. The GCP client
/// libraries handle refreshing automatically, so we keep this short.
const TOKEN_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// Provides GCP Workload Identity Federation credentials for the agent session.
///
/// The credential config file is written eagerly during construction. GCP SDKs
/// discover it via `GOOGLE_APPLICATION_CREDENTIALS` and invoke the embedded
/// executable to obtain tokens on demand.
pub(crate) struct GcpCloudProvider {
    credentials: GcpCredentials,
}

impl GcpCloudProvider {
    const PROVIDER_NAME: &'static str = "gcp";

    pub fn new(config: &GcpProviderConfig, run_id: &str) -> Result<Self> {
        let federation_config = GcpFederationConfig {
            project_number: config.project_number.clone(),
            pool_id: config.workload_identity_federation_pool_id.clone(),
            provider_id: config.workload_identity_federation_provider_id.clone(),
            service_account_email: config.service_account_email.clone(),
            token_lifetime: Some(TOKEN_LIFETIME),
        };

        let credentials = GcpCredentials::federated(run_id, &federation_config)
            .context("Failed to prepare GCP federation credentials")
            .map_err(|error| CloudProviderSetupError::new(Self::PROVIDER_NAME, error))?;

        Ok(Self { credentials })
    }
}

impl CloudProvider for GcpCloudProvider {
    fn env_vars(&self) -> Result<HashMap<OsString, OsString>> {
        Ok(self.credentials.env_vars())
    }

    fn cleanup(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        Box::pin(async move {
            self.credentials
                .cleanup()
                .context("Failed to remove GCP credential files")
                .map_err(|err| CloudProviderSetupError::new(Self::PROVIDER_NAME, err))
        })
    }
}
