use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context as _;
use command::r#async::Command;
use warp_core::{safe_info, safe_warn};
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

/// Outcome of running a best-effort external command (such as `gcloud auth login`)
/// with a timeout. Spawn failures and non-zero exits are reported rather than
/// panicked on, since the surrounding setup can still succeed without the command.
#[derive(Debug)]
enum BestEffortOutcome {
    /// The command exited successfully.
    Success,
    /// The command exited with a non-zero status. `stderr` is captured for
    /// dogfood-only diagnostics; it is never logged in release channels.
    NonZeroExit {
        status: std::process::ExitStatus,
        stderr: String,
    },
    /// The command binary was not found (e.g. `gcloud` isn't installed).
    NotFound,
    /// The command could not be spawned for any other reason.
    SpawnFailed(std::io::Error),
    /// The command did not finish within the timeout and was killed.
    Timeout,
}

/// Runs `command` to completion, waiting up to `timeout`.
///
/// The command is run with `kill_on_drop(true)` so that if it does not finish
/// within `timeout`, dropping the wait future on timeout kills the spawned
/// process rather than leaving it running after the caller continues. This
/// matches the `with_timeout` pattern used elsewhere (e.g. `ssh -O exit`):
/// `with_timeout` only stops waiting on the `output()` future, so without
/// `kill_on_drop` a hung child would outlive the timeout.
async fn run_best_effort_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> BestEffortOutcome {
    command.kill_on_drop(true);
    match command.output().with_timeout(timeout).await {
        Ok(Ok(output)) if output.status.success() => BestEffortOutcome::Success,
        Ok(Ok(output)) => BestEffortOutcome::NonZeroExit {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        },
        Ok(Err(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            BestEffortOutcome::NotFound
        }
        Ok(Err(err)) => BestEffortOutcome::SpawnFailed(err),
        Err(_timeout) => BestEffortOutcome::Timeout,
    }
}

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

            match run_best_effort_with_timeout(command, GCLOUD_LOGIN_TIMEOUT).await {
                BestEffortOutcome::Success => {
                    log::info!("gcloud auth login succeeded for GCP cloud provider");
                }
                BestEffortOutcome::NonZeroExit { status, stderr } => {
                    // `gcloud` ran but returned a non-zero status. The ADC env vars still
                    // work, so log and continue rather than failing setup. The stderr comes
                    // from an external auth tool and may include credential paths or account
                    // identifiers, so it is kept out of the release `safe:` arm (which
                    // becomes a breadcrumb) and only appears in the dogfood `full:` arm.
                    let stderr_trimmed = stderr.trim();
                    safe_warn!(
                        safe: ("gcloud auth login exited with non-zero status; continuing (ADC env vars still provide credentials)"),
                        full: ("gcloud auth login exited with status {status}: {stderr_trimmed}; continuing (ADC env vars still provide credentials)")
                    );
                }
                BestEffortOutcome::NotFound => {
                    // `gcloud` isn't installed. This is expected and non-fatal: we don't
                    // require the CLI, and the ADC env vars still provide credentials.
                    log::info!("gcloud not found; skipping gcloud auth login");
                }
                BestEffortOutcome::SpawnFailed(err) => {
                    safe_warn!(
                        safe: ("Failed to spawn gcloud auth login; continuing"),
                        full: ("Failed to spawn gcloud auth login: {err}; continuing")
                    );
                }
                BestEffortOutcome::Timeout => {
                    log::warn!(
                        "gcloud auth login timed out after {GCLOUD_LOGIN_TIMEOUT:?}; killed the process and continuing"
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

#[cfg(test)]
#[path = "gcp_tests.rs"]
mod tests;
