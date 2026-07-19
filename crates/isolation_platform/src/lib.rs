use std::io;
use std::process::ExitStatus;
use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Serialize;
use warp_core::channel::{Channel, ChannelState};

#[cfg(not(target_family = "wasm"))]
mod docker;
#[cfg(not(target_family = "wasm"))]
mod docker_sandbox;
#[cfg(not(target_family = "wasm"))]
mod kubernetes;
/// Namespace isolation-platform integration.
#[cfg(not(target_family = "wasm"))]
pub mod namespace;

/// Environment variable set by the server to identify the isolation platform.
/// The value should match one of the `IsolationPlatformType` variants in snake_case.
#[cfg(not(target_family = "wasm"))]
const WARP_ISOLATION_PLATFORM_ENV: &str = "WARP_ISOLATION_PLATFORM";

/// Environment variable containing the generic Warp-managed workload token that we use
/// for isolation platforms that don't issue their own tokens.
#[cfg(not(target_family = "wasm"))]
const WARP_WORKLOAD_TOKEN_ENV: &str = "WARP_WORKLOAD_TOKEN";

/// A kind of isolation platform. For our usage, isolation platforms are different ways where Warp
/// can be sandboxed, such as VMs, containers, or cloud hosts. This may also include weaker forms
/// of sandboxing such as Git worktrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationPlatformType {
    /// Warp is running within a Docker container. Note that this does *not* mean this is a Warp-hosted
    /// Docker Sandboxes environment. Instead, it's likely a self-hosted agent.
    #[cfg(not(target_family = "wasm"))]
    Docker,
    /// Warp is running within a Docker Sandbox, likely as a Warp-hosted agent.
    #[cfg(not(target_family = "wasm"))]
    DockerSandbox,
    /// Warp is running within a Kubernetes pod, likely as a self-hosted agent.
    #[cfg(not(target_family = "wasm"))]
    Kubernetes,
    /// Warp is running within a Namespace instance, likely as a Warp-hosted agent.
    #[cfg(not(target_family = "wasm"))]
    Namespace,
}

/// A workload identity token issued by the isolation platform.
#[derive(Debug, Clone)]
pub struct WorkloadToken {
    /// The token string.
    pub token: String,
    /// The expiration time of the token. On some platforms, workload tokens do not expire.
    pub expires_at: Option<DateTime<Utc>>,
}

/// Detect the current isolation platform, if any.
///
/// Results are memoized for the lifetime of the process.
pub fn detect() -> Option<IsolationPlatformType> {
    static DETECTED_PLATFORM: OnceLock<Option<IsolationPlatformType>> = OnceLock::new();

    *DETECTED_PLATFORM.get_or_init(|| {
        // This never applies to integration tests.
        if ChannelState::channel() == Channel::Integration {
            return None;
        }

        // Use a closure so we can early-return.
        #[allow(clippy::redundant_closure_call)]
        let platform = (|| {
            // If the server explicitly told us which platform we're on, trust it.
            // This takes priority over all heuristic-based detection.
            #[cfg(not(target_family = "wasm"))]
            if let Some(platform) = platform_from_env() {
                return Some(platform);
            }

            #[cfg(not(target_family = "wasm"))]
            if namespace::is_in_namespace_instance() {
                return Some(IsolationPlatformType::Namespace);
            }

            #[cfg(not(target_family = "wasm"))]
            if kubernetes::is_in_kubernetes() {
                return Some(IsolationPlatformType::Kubernetes);
            }

            #[cfg(not(target_family = "wasm"))]
            if docker::is_in_docker() {
                return Some(IsolationPlatformType::Docker);
            }

            None
        })();

        match platform {
            Some(platform) => {
                log::debug!("Detected isolation platform: {:?}", platform);
            }
            None => {
                log::info!("No isolation platform detected");
            }
        }

        platform
    })
}

/// Issue a workload identity token for the current isolation platform.
///
/// This will fail if no isolation platform is detected and no platform-agnostic workload token
/// is available.
#[cfg_attr(target_family = "wasm", allow(unused_variables))]
pub async fn issue_workload_token(
    duration: Option<Duration>,
) -> Result<WorkloadToken, IsolationPlatformError> {
    match detect() {
        #[cfg(not(target_family = "wasm"))]
        Some(IsolationPlatformType::DockerSandbox) => {
            docker_sandbox::issue_workload_token(duration).await
        }
        #[cfg(not(target_family = "wasm"))]
        Some(IsolationPlatformType::Namespace) => namespace::issue_workload_token(duration).await,
        #[cfg(not(target_family = "wasm"))]
        // Check for a platform-agnostic workload token if there's no
        // isolation platform or if the detected platform doesn't have
        // its own workload token mechanism.
        _ => read_generic_workload_token()
            .inspect_err(|err| log::debug!("No platform-agnostic workload token: {err}"))
            .map_err(|_| IsolationPlatformError::NoIsolationPlatformDetected),
        #[cfg(target_family = "wasm")]
        _ => Err(IsolationPlatformError::NoIsolationPlatformDetected),
    }
}

/// Read a platform-agnostic workload token from the `WARP_WORKLOAD_TOKEN` environment variable.
/// Returns a `WorkloadToken` with no expiration, or an error if the variable is missing/empty.
#[cfg(not(target_family = "wasm"))]
fn read_generic_workload_token() -> Result<WorkloadToken, IsolationPlatformError> {
    let token = std::env::var(WARP_WORKLOAD_TOKEN_ENV)
        .map_err(|_| IsolationPlatformError::GenericWorkloadTokenMissing)?;
    if token.is_empty() {
        return Err(IsolationPlatformError::GenericWorkloadTokenMissing);
    }
    Ok(WorkloadToken {
        token,
        expires_at: None,
    })
}

/// Parse the `WARP_ISOLATION_PLATFORM` environment variable into a platform type.
#[cfg(not(target_family = "wasm"))]
fn platform_from_env() -> Option<IsolationPlatformType> {
    let value = std::env::var(WARP_ISOLATION_PLATFORM_ENV).ok()?;
    match value.as_str() {
        "docker" => Some(IsolationPlatformType::Docker),
        "docker_sandbox" => Some(IsolationPlatformType::DockerSandbox),
        "kubernetes" => Some(IsolationPlatformType::Kubernetes),
        "namespace" => Some(IsolationPlatformType::Namespace),
        other => {
            log::warn!("Unknown {WARP_ISOLATION_PLATFORM_ENV} value: {other}");
            None
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IsolationPlatformError {
    #[error("No isolation platform detected")]
    NoIsolationPlatformDetected,

    #[error("Workload token is missing or empty")]
    GenericWorkloadTokenMissing,

    #[error("Required command {command} is unavailable")]
    CommandUnavailable {
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("Command `{command}` exited with non-zero status: {status}")]
    CommandFailed { command: String, status: ExitStatus },

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
