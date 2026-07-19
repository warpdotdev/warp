use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use build_cache::{CacheInvocationScope, CacheSetupOutcome, PlatformCacheMode, Repository};
use warp_completer::completer::CommandExitStatus;
use warp_isolation_platform::IsolationPlatformType;
use warpui::ModelSpawner;

use super::terminal::TerminalDriver;
use crate::ai::agent_sdk::setup_observability::NamespaceCacheMountReport;
use crate::ai::cloud_environments::SourceRepo;
use crate::server::server_api::ai::{
    AgentRunClientCacheInvocationPayload, AgentRunClientCacheModePayload,
};
use crate::terminal::model::session::command_executor::shell_escape_single_quotes;
use crate::terminal::shell::ShellType;
use crate::util::path::resolve_executable;

pub(super) const BUILD_CACHE_ROOT_ENV: &str = build_cache::BUILD_CACHE_ROOT_ENV;

#[derive(Debug, thiserror::Error)]
enum CacheIntegrationError {
    #[error("cache operation failed: {0}")]
    Operation(String),
    #[error("cache environment variable name is invalid")]
    InvalidEnvironmentName,
}

pub(super) fn build_cache_root() -> Option<PathBuf> {
    if warp_isolation_platform::detect() != Some(IsolationPlatformType::Namespace) {
        return None;
    }
    validated_build_cache_root(std::env::var_os(BUILD_CACHE_ROOT_ENV)?)
}

fn validated_build_cache_root(value: OsString) -> Option<PathBuf> {
    let root = PathBuf::from(value);
    (!root.as_os_str().is_empty() && root.is_absolute()).then_some(root)
}

pub(super) async fn setup_namespace_caches(
    repos: &[SourceRepo],
    working_dir: &Path,
    build_root: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> NamespaceCacheMountReport {
    let repositories = repos
        .iter()
        .map(|repo| {
            Repository::new(
                repo.code_forge.unwrap_or_default().host(),
                &repo.owner,
                &repo.repo,
                working_dir.join(&repo.repo),
            )
        })
        .collect::<Vec<_>>();
    let mut outcome = build_cache::setup(&repositories, build_root, platform_cache_mode()).await;
    if let Err(error) = export_environment(&outcome.environment, spawner).await {
        log::warn!("Failed to export Namespace cache environment: {error}");
        outcome.mark_error();
    }
    into_event_report(outcome)
}

fn platform_cache_mode() -> Option<PlatformCacheMode> {
    #[cfg(target_os = "linux")]
    {
        resolve_executable("apt-config")
            .is_some()
            .then_some(PlatformCacheMode::Apt)
    }
    #[cfg(target_os = "macos")]
    {
        resolve_executable("brew")
            .is_some()
            .then_some(PlatformCacheMode::Brew)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

async fn export_environment(
    environment: &BTreeMap<String, String>,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), CacheIntegrationError> {
    let shell_type = spawner
        .spawn(|driver, ctx| {
            driver
                .active_session_shell_type(ctx)
                .unwrap_or(ShellType::Bash)
        })
        .await
        .map_err(|error| CacheIntegrationError::Operation(error.to_string()))?;
    let command = build_export_command(environment, shell_type)?;
    let output = spawner
        .spawn(move |driver, ctx| driver.execute_silent_command(command, ctx))
        .await
        .map_err(|error| CacheIntegrationError::Operation(error.to_string()))?
        .await
        .map_err(|error| CacheIntegrationError::Operation(error.to_string()))?;
    if output.status != CommandExitStatus::Success {
        return Err(CacheIntegrationError::Operation(
            "session environment export command failed".to_owned(),
        ));
    }
    Ok(())
}

fn build_export_command(
    environment: &BTreeMap<String, String>,
    shell_type: ShellType,
) -> Result<String, CacheIntegrationError> {
    if environment.is_empty() {
        return Ok(":".to_owned());
    }

    environment
        .iter()
        .map(|(name, value)| {
            if !is_valid_environment_name(name) {
                return Err(CacheIntegrationError::InvalidEnvironmentName);
            }
            let value = shell_escape_single_quotes(value, shell_type);
            Ok(format!("export {name}='{value}'"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|commands| commands.join("\n"))
}

fn is_valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn into_event_report(outcome: CacheSetupOutcome) -> NamespaceCacheMountReport {
    let invocations = outcome
        .invocations
        .into_iter()
        .map(|invocation| {
            let modes = invocation
                .modes
                .into_iter()
                .map(|(name, report)| {
                    AgentRunClientCacheModePayload::new(
                        name,
                        report.cache_hits,
                        report.cache_misses,
                    )
                })
                .collect();
            match invocation.scope {
                CacheInvocationScope::Shared => {
                    AgentRunClientCacheInvocationPayload::shared(invocation.is_error, modes)
                }
                CacheInvocationScope::Repository { repo_key } => {
                    AgentRunClientCacheInvocationPayload::repository(
                        repo_key,
                        invocation.is_error,
                        modes,
                    )
                }
            }
        })
        .collect();
    NamespaceCacheMountReport::new(outcome.setup_is_error, invocations)
}

#[cfg(test)]
#[path = "namespace_cache_tests.rs"]
mod tests;
