use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context as _;
use command::r#async::Command;
use warp_core::safe_info;
use warp_managed_secrets::{GcpCredentials, GcpFederationConfig};
use warpui::ModelSpawner;
use warpui::r#async::FutureExt as _;

use super::super::terminal::TerminalDriver;
use super::{CloudProvider, CloudProviderSetupError, Result};
use crate::ai::cloud_environments::GcpProviderConfig;

/// Token lifetime for GCP executable-sourced credentials. The GCP client
/// libraries handle refreshing automatically, so we keep this short.
const TOKEN_LIFETIME: Duration = Duration::from_secs(30 * 60);

/// Upper bound on how long we wait for `gcloud auth login` to complete. This is
/// a best-effort convenience step, so we cap it to avoid wedging setup.
const GCLOUD_LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

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

    fn setup(
        &mut self,
        _spawner: ModelSpawner<TerminalDriver>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            // Point `gcloud`'s own auth system at the federated credential file. Writing
            // the config file and exporting `GOOGLE_APPLICATION_CREDENTIALS` is enough for
            // the GCP SDKs, but `gcloud` needs an explicit `auth login` for it to report an
            // active account, which some tooling depends on for full functionality.
            //
            // This is best-effort: `gcloud` may not be installed, so a spawn failure
            // (notably `NotFound`) is logged and ignored rather than failing setup.
            let config_file_path = self.credentials.config_file_path();
            safe_info!(
                safe: ("Activating gcloud auth for GCP cloud provider credentials"),
                full: ("Activating gcloud auth with cred-file {}", config_file_path.display())
            );

            let mut command = Command::new("gcloud");
            command
                // Ensure `gcloud` is allowed to invoke the executable-sourced credential
                // command when it validates the account during login.
                .envs(self.credentials.env_vars())
                .arg("--quiet")
                .arg("auth")
                .arg("login")
                .arg("--force")
                .arg("--cred-file")
                .arg(config_file_path);

            match command.output().with_timeout(GCLOUD_LOGIN_TIMEOUT).await {
                Ok(Ok(output)) if output.status.success() => {
                    log::info!("gcloud auth login succeeded for GCP cloud provider");
                }
                Ok(Ok(output)) => {
                    // `gcloud` ran but returned a non-zero status. The ADC env vars still
                    // work, so log and continue rather than failing setup.
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    log::warn!(
                        "gcloud auth login exited with status {}: {}",
                        output.status,
                        stderr.trim()
                    );
                }
                Ok(Err(err)) if err.kind() == std::io::ErrorKind::NotFound => {
                    // `gcloud` isn't installed. This is expected and non-fatal: we don't
                    // require the CLI, and the ADC env vars still provide credentials.
                    log::info!("gcloud not found; skipping gcloud auth login");
                }
                Ok(Err(err)) => {
                    log::warn!("Failed to spawn gcloud auth login: {err}");
                }
                Err(_timeout) => {
                    log::warn!(
                        "gcloud auth login timed out after {GCLOUD_LOGIN_TIMEOUT:?}; continuing"
                    );
                }
            }

            Ok(())
        })
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
