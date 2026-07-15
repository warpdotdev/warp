use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use build_cache::{CacheSetupError, CacheSetupReport, RepoIdentity, RepositoryCacheSource};
use cloud_object_models::SourceRepo;
use warp_completer::completer::{CommandExitStatus, CommandOutput};
use warp_errors::report_error;
use warp_isolation_platform::IsolationPlatformType;
use warpui::ModelSpawner;

use super::terminal::TerminalDriver;

const BUILD_CACHE_ROOT_ENV: &str = "WARP_BUILD_CACHE_ROOT";

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("build cache setup completed with degraded results")]
pub(crate) struct CacheSetupDegraded;

pub(crate) fn should_setup_cache(
    platform: Option<IsolationPlatformType>,
    cache_root: Option<&OsStr>,
) -> bool {
    platform == Some(IsolationPlatformType::Namespace)
        && cache_root.is_some_and(|root| !root.is_empty())
}

pub(crate) fn enabled_cache_root() -> Option<PathBuf> {
    let root = std::env::var_os(BUILD_CACHE_ROOT_ENV);
    if should_setup_cache(warp_isolation_platform::detect(), root.as_deref()) {
        root.map(Into::into)
    } else {
        None
    }
}

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
    if let Some(script) = report.export_script {
        if apply_export_with(script, |script| async move {
            spawner
                .spawn(move |driver, ctx| driver.execute_silent_command(script, ctx))
                .await
                .map_err(|_| CacheSetupError::EnvExportFailed)?
                .await
                .map_err(|_| CacheSetupError::EnvExportFailed)
        })
        .await
        .is_err()
        {
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

async fn apply_export_with<F, Fut>(script: String, execute: F) -> Result<(), CacheSetupError>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<CommandOutput, CacheSetupError>>,
{
    let output = execute(script).await?;
    if output.status == CommandExitStatus::Success {
        Ok(())
    } else {
        Err(CacheSetupError::EnvExportFailed)
    }
}

#[cfg(test)]
#[path = "cache_setup_tests.rs"]
mod tests;
