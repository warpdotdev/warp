// spacectl execution intentionally uses std::process::Command for the build_cache crate boundary.
#![allow(clippy::disallowed_types)]
use std::ffi::OsStr;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use build_cache::{
    CacheSetupError, CacheSetupReport, HostPlatform, InvocationReport, RepoIdentity,
    RepositoryCacheSource, SystemCacheTools,
};
use cloud_object_models::SourceRepo;
use is_executable::IsExecutable as _;
use warp_completer::completer::{CommandExitStatus, CommandOutput};
use warp_errors::report_error;
use warp_isolation_platform::IsolationPlatformType;
use warpui::ModelSpawner;

use super::terminal::TerminalDriver;

const BUILD_CACHE_ROOT_ENV: &str = "WARP_BUILD_CACHE_ROOT";
const SPACECTL_TIMEOUT: Duration = Duration::from_secs(60);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);

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
        current_platform(),
        current_system_cache_tools(),
        |command| async move {
            blocking::unblock(move || run_command_with_timeout(command, SPACECTL_TIMEOUT)).await
        },
    )
    .await;

    let mut degraded = report_degradations(&report);
    if let Some(script) = report.export_script {
        if apply_export(script, spawner).await.is_err() {
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

fn current_platform() -> HostPlatform {
    if cfg!(target_os = "linux") {
        HostPlatform::Linux
    } else if cfg!(target_os = "macos") {
        HostPlatform::MacOs
    } else {
        HostPlatform::Other
    }
}

fn current_system_cache_tools() -> SystemCacheTools {
    SystemCacheTools {
        apt_config: command_resolves_on_path("apt-config"),
        brew: command_resolves_on_path("brew"),
    }
}

fn command_resolves_on_path(command: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(command).is_executable())
    })
}

fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<Vec<u8>, CacheSetupError> {
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| CacheSetupError::SpawnFailed)?;
    let stdout = child.stdout.take().ok_or(CacheSetupError::SpawnFailed)?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::BufReader::new(stdout).read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = stdout_reader
                    .join()
                    .map_err(|_| CacheSetupError::SpawnFailed)?;
                return if status.success() {
                    Ok(bytes)
                } else {
                    Err(CacheSetupError::NonzeroExit {
                        exit_code: status.code(),
                    })
                };
            }
            Ok(None) if started.elapsed() < timeout => {
                thread::sleep(PROCESS_POLL_INTERVAL);
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                return Err(CacheSetupError::Timeout);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                return Err(CacheSetupError::SpawnFailed);
            }
        }
    }
}

fn report_degradations(report: &CacheSetupReport) -> bool {
    let mut degraded = false;
    report_degradations_with(report, |error, invocation| {
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
    });
    degraded
}

fn report_degradations_with(
    report: &CacheSetupReport,
    mut reporter: impl FnMut(&CacheSetupError, &InvocationReport),
) {
    for invocation in report.degradations() {
        if let Some(error) = &invocation.error {
            reporter(error, invocation);
        }
    }
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

async fn apply_export(
    script: String,
    spawner: &ModelSpawner<TerminalDriver>,
) -> Result<(), CacheSetupError> {
    apply_export_with(script, |script| async move {
        spawner
            .spawn(move |driver, ctx| driver.execute_silent_command(script, ctx))
            .await
            .map_err(|_| CacheSetupError::EnvExportFailed)?
            .await
            .map_err(|_| CacheSetupError::EnvExportFailed)
    })
    .await
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
