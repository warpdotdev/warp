use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use build_cache::{CacheSetupError, CacheSetupReport, RepoIdentity, RepositoryCacheSource};
use cloud_object_models::SourceRepo;
use warp_completer::completer::CommandExitStatus;
use warp_errors::report_error;
use warp_isolation_platform::IsolationPlatformType;
use warpui::ModelSpawner;

use super::terminal::TerminalDriver;
use crate::terminal::model::session::command_executor::shell_escape_single_quotes;
use crate::terminal::shell::ShellType;

const BUILD_CACHE_ROOT_ENV: &str = "WARP_BUILD_CACHE_ROOT";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("build cache setup completed with degraded results")]
pub(crate) struct CacheSetupDegraded;

/// Returns whether or not build caching should be set up for the given platform and environment-sourced
/// cache location.
pub(crate) fn should_setup_cache(
    platform: Option<IsolationPlatformType>,
    cache_root: Option<&OsStr>,
) -> bool {
    platform == Some(IsolationPlatformType::Namespace)
        && cache_root.is_some_and(|root| !root.is_empty())
}

/// Returns the persistent cache location, if enabled.
pub(crate) fn enabled_cache_root() -> Option<PathBuf> {
    let root = std::env::var_os(BUILD_CACHE_ROOT_ENV);
    if should_setup_cache(warp_isolation_platform::detect(), root.as_deref()) {
        root.map(Into::into)
    } else {
        None
    }
}

/// Convert a [`SourceRepo`] from the configured environment into the [`build_cache`] crate's
/// [`RepositoryCacheSource`] type.
pub(crate) fn repository_cache_source(
    repo: &SourceRepo,
    working_dir: &Path,
) -> RepositoryCacheSource {
    let forge_host = repo.code_forge.unwrap_or_default().host();
    RepositoryCacheSource {
        name: format!("{}/{}", repo.owner, repo.repo),
        identity: RepoIdentity::new(forge_host, &repo.owner, &repo.repo),
        cwd: working_dir.join(&repo.repo),
    }
}

/// Set up build caching for all provided repositories, assuming they're cloned into the given
/// working directory. This will modify the filesystem, including some locations outside the
/// cloned repositories. If any caches require environment variables, they are applied to the
/// given terminal session.
pub(crate) async fn setup_caches(
    cache_root: PathBuf,
    source_repos: &[SourceRepo],
    working_dir: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), CacheSetupDegraded> {
    let repositories = source_repos
        .iter()
        .map(|repo| repository_cache_source(repo, working_dir))
        .collect();
    let report = build_cache::setup_cache(
        cache_root,
        repositories,
        build_cache::global_cache_modes(),
        build_cache::default_run_command,
    )
    .await;

    let mut degraded = report_degradations(&report);

    if !report.add_envs.is_empty() {
        let add_envs = report.add_envs;
        let output = match spawner
            .spawn(move |driver, ctx| {
                let shell_type = driver
                    .active_session_shell_type(ctx)
                    .unwrap_or(ShellType::Bash);
                let command = build_export_command(&add_envs, shell_type);
                driver.execute_silent_command(command, ctx)
            })
            .await
        {
            Ok(output) => output.await.map_err(|_| CacheSetupError::EnvExportFailed),
            Err(_) => Err(CacheSetupError::EnvExportFailed),
        };
        if !matches!(output, Ok(output) if output.status == CommandExitStatus::Success) {
            let error = CacheSetupError::EnvExportFailed;
            report_cache_error(&error, "global", "", "", None, Duration::ZERO);
            degraded = true;
        }
    }

    if degraded {
        Err(CacheSetupDegraded)
    } else {
        Ok(())
    }
}

/// Report any cache setup failures to Sentry.
fn report_degradations(report: &CacheSetupReport) -> bool {
    let mut degraded = false;
    for invocation in report.degradations() {
        if let Some(error) = &invocation.error {
            let repo_key = invocation
                .scope
                .repo_key()
                .map(ToString::to_string)
                .unwrap_or_default();
            let modes = invocation.modes.join(",");
            report_cache_error(
                error,
                invocation.scope.kind(),
                &repo_key,
                &modes,
                error.exit_code(),
                invocation.duration,
            );
            degraded = true;
        }
    }
    degraded
}

fn report_cache_error(
    error: &CacheSetupError,
    scope: &'static str,
    repo_key: &str,
    modes: &str,
    exit_code: Option<i32>,
    duration: Duration,
) {
    // If a fixed failure category becomes noisy, add ReportErrorLogMode::OncePerRun here.
    report_error!(
        error,
        extra: {
            "scope" => scope,
            "repo_key" => repo_key,
            "mode" => modes,
            "error_kind" => error.kind(),
            "exit_code" => ?exit_code,
            "duration_ms" => %duration.as_millis()
        }
    );
    log::warn!(
        "Build cache setup degraded: scope={scope} error_kind={}",
        error.kind()
    );
}

/// Build a shell-safe command to export environment variables.
fn build_export_command(environment: &BTreeMap<String, String>, shell_type: ShellType) -> String {
    environment
        .iter()
        .map(|(name, value)| {
            let value = shell_escape_single_quotes(value, shell_type);
            match shell_type {
                ShellType::Bash | ShellType::Zsh => format!("export {name}='{value}'"),
                ShellType::Fish => format!("set -gx {name} '{value}'"),
                ShellType::PowerShell => format!("$env:{name} = '{value}'"),
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
#[path = "cache_setup_tests.rs"]
mod tests;
