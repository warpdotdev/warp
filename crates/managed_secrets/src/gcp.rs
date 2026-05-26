use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::NamedTempFile;
use warp_core::safe_debug;

use crate::client::TaskIdentityToken;

const GCP_WORKLOAD_IDENTITY_FEDERATION_VERSION: u8 = 1;
pub(crate) const TOKEN_TYPE_ID_TOKEN: &str = "urn:ietf:params:oauth:token-type:id_token";
pub(crate) const TOKEN_TYPE_JWT: &str = "urn:ietf:params:oauth:token-type:jwt";

/// Configuration for GCP Workload Identity Federation.
///
/// These fields map directly to the GCP concepts required to set up
/// executable-sourced external credentials
/// ([AIP-4117](https://google.aip.dev/auth/4117)).
#[derive(Debug, Clone)]
pub struct GcpFederationConfig {
    /// GCP project number (not project ID).
    pub project_number: String,
    /// Workload identity pool ID.
    pub pool_id: String,
    /// Workload identity pool provider ID.
    pub provider_id: String,
    /// Service account email for impersonation. When set, the federated token
    /// is exchanged for a service account access token.
    pub service_account_email: Option<String>,
    /// Lifetime for the impersonated service account token.
    pub token_lifetime: Option<Duration>,
}

/// Handle for GCP Workload Identity Federation credentials.
///
/// The handle represents a GCP authentication config file that uses Workload
/// Identity Federation to authenticate to GCP as a particular Oz agent run.
///
/// When the handle is dropped, the backing temporary files are deleted.
pub struct GcpCredentials {
    /// Temporary file holding the GCP credentials configuration file.
    config_file: NamedTempFile,
    /// Temporary file where Warp OIDC tokens are cached.
    output_file: NamedTempFile,
}

impl GcpCredentials {
    /// Create executable-sourced GCP federation credentials for the given task.
    ///
    /// This writes an [AIP-4117](https://google.aip.dev/auth/4117)
    /// `external_account` config to a temporary file and returns the
    /// environment variables required for ADC to discover it.
    pub fn federated(
        task_id: &str,
        config: &GcpFederationConfig,
    ) -> Result<Self, PrepareGcpCredentialsError> {
        safe_debug!(
            safe: ("Configuring GCP workload identity federation"),
            full: ("Configuring GCP workload identity federation for project={}, pool={}, provider={}", config.project_number, config.pool_id, config.provider_id)
        );

        let oz_binary_path =
            std::env::current_exe().map_err(|_| PrepareGcpCredentialsError::NoBinaryPath)?;

        // Create the output file that the executable will write cached tokens to.
        let output_file = NamedTempFile::new()
            .map_err(|source| PrepareGcpCredentialsError::FileCreate { source })?;

        let cred_config_json =
            generate_gcp_credential_config(task_id, config, &oz_binary_path, output_file.path())?;
        safe_debug!(
            safe: ("Generated federated GCP configuration"),
            full: ("Generated federated GCP configuration: {cred_config_json:#}")
        );

        let json_bytes = serde_json::to_vec_pretty(&cred_config_json)
            .map_err(PrepareGcpCredentialsError::SerializeConfig)?;

        let mut config_file = NamedTempFile::new()
            .map_err(|source| PrepareGcpCredentialsError::FileCreate { source })?;
        config_file.write_all(&json_bytes).map_err(|source| {
            PrepareGcpCredentialsError::FileWrite {
                path: config_file.path().to_path_buf(),
                source,
            }
        })?;
        safe_debug!(
            safe: ("Wrote GCP credentials config file"),
            full: ("Wrote GCP credentials to {}", config_file.path().display())
        );

        Ok(Self {
            config_file,
            output_file,
        })
    }

    /// Environment variables to set in a session in order to use this GCP
    /// configuration.
    pub fn env_vars(&self) -> HashMap<OsString, OsString> {
        let config_file_path = self.config_file.path().as_os_str();
        let mut vars = HashMap::with_capacity(3);
        vars.insert(
            OsString::from("GOOGLE_EXTERNAL_ACCOUNT_ALLOW_EXECUTABLES"),
            OsString::from("1"),
        );
        // Google Cloud SDKs use the `GOOGLE_APPLICATION_CREDENTIALS` variable.
        vars.insert(
            OsString::from("GOOGLE_APPLICATION_CREDENTIALS"),
            config_file_path.to_owned(),
        );
        // The `gcloud` CLI has its own auth system, but accepts credential file overrides.
        vars.insert(
            OsString::from("CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE"),
            config_file_path.to_owned(),
        );
        vars
    }

    /// Clean up the GCP credential state. This will remove the temporary configuration and
    /// token files.
    pub fn cleanup(self) -> std::io::Result<()> {
        self.config_file.close()?;
        self.output_file.close()?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareGcpCredentialsError {
    #[error("Could not determine the current executable path")]
    NoBinaryPath,
    #[error("Cannot use executable {} for GCP executable-sourced credentials", path.display())]
    InvalidBinaryPath { path: PathBuf },
    #[error("Cannot use run {task_id} for GCP executable-sourced credentials")]
    InvalidTaskId { task_id: String },
    #[error("Failed to create credential config file: {source}")]
    FileCreate {
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to serialize credential config: {0}")]
    SerializeConfig(#[source] serde_json::Error),
    #[error("Failed to write credential config to {path}: {source}")]
    FileWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Successful output payload for GCP executable-sourced credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GcpWorkloadIdentityFederationToken {
    pub version: u8,
    pub success: bool,
    pub token_type: String,
    pub id_token: String,
    pub expiration_time: i64,
}

impl GcpWorkloadIdentityFederationToken {
    pub(crate) fn new(token: TaskIdentityToken, token_type: String) -> Self {
        Self {
            version: GCP_WORKLOAD_IDENTITY_FEDERATION_VERSION,
            success: true,
            token_type,
            id_token: token.token,
            expiration_time: token.expires_at.timestamp(),
        }
    }
}

/// Error output payload for GCP executable-sourced credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GcpWorkloadIdentityFederationError {
    pub version: u8,
    pub success: bool,
    pub code: String,
    pub message: String,
}

impl GcpWorkloadIdentityFederationError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            version: GCP_WORKLOAD_IDENTITY_FEDERATION_VERSION,
            success: false,
            code: "TOKEN_ISSUANCE_FAILED".into(),
            message: message.into(),
        }
    }
}

pub(crate) fn gcp_workload_identity_federation_audience(
    project_number: &str,
    pool_id: &str,
    provider_id: &str,
) -> String {
    format!(
        "//iam.googleapis.com/projects/{project_number}/locations/global/workloadIdentityPools/{pool_id}/providers/{provider_id}"
    )
}

/// Produce the [AIP-4117](https://google.aip.dev/auth/4117) `external_account`
/// credential configuration JSON.
///
/// The returned value can be written to
/// `$HOME/.config/gcloud/application_default_credentials.json` so that any GCP
/// SDK picks it up automatically.
///
/// `oz_binary_path` should be the absolute path to the current `oz` executable.
fn generate_gcp_credential_config(
    task_id: &str,
    config: &GcpFederationConfig,
    oz_binary_path: &Path,
    output_file: &Path,
) -> Result<Value, PrepareGcpCredentialsError> {
    let audience = gcp_workload_identity_federation_audience(
        &config.project_number,
        &config.pool_id,
        &config.provider_id,
    );

    let oz_binary_display = oz_binary_path.display().to_string();
    // The executable command is embedded as a single string in the credential
    // config. GCP SDKs split it with `strings.Fields` (Go) or `shlex.split`
    // (Python), so whitespace in either the binary path or the task ID would
    // cause the command to be misparsed.
    if oz_binary_display.contains(' ') {
        return Err(PrepareGcpCredentialsError::InvalidBinaryPath {
            path: oz_binary_path.to_path_buf(),
        });
    }
    if task_id.contains(' ') {
        return Err(PrepareGcpCredentialsError::InvalidTaskId {
            task_id: task_id.to_owned(),
        });
    }

    let mut command = format!("{oz_binary_display} federate issue-gcp-token --run-id {task_id}");
    if let Some(lifetime) = config.token_lifetime {
        command.push_str(&format!(" --duration {}s", lifetime.as_secs()));
    }

    let mut cred_config = json!({
        "type": "external_account",
        "audience": audience,
        "subject_token_type": "urn:ietf:params:oauth:token-type:id_token",
        // Regional STS endpoints with workload identity federation are pre-GA and we do not yet
        // support them:
        // https://docs.cloud.google.com/iam/docs/best-practices-for-using-workload-identity-federation#sts-regional-endpoints
        "token_url": "https://sts.googleapis.com/v1/token",
        "credential_source": {
            "executable": {
                "command": command,
                "timeout_millis": 30000,
                "output_file": output_file.display().to_string()
            }
        }
    });

    if let Some(email) = &config.service_account_email {
        let impersonation_url = format!(
            "https://iamcredentials.googleapis.com/v1/projects/-/serviceAccounts/{email}:generateAccessToken"
        );
        cred_config["service_account_impersonation_url"] = json!(impersonation_url);

        if let Some(lifetime) = config.token_lifetime {
            cred_config["service_account_impersonation"] = json!({
                "token_lifetime_seconds": lifetime.as_secs()
            });
        }
    }

    Ok(cred_config)
}

#[cfg(test)]
#[path = "gcp_tests.rs"]
mod tests;
