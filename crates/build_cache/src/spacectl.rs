//! Wrappers for [`spacectl`](https://github.com/namespacelabs/spacectl). `spacectl` is a CLI
//! provided by Namespace which powers various integrations within Namespace instances.
//!
//! We use it to configure build caching - among other things, `spacectl` can:
//! * Detect the tools used by a given codebase (such as Gradle, Ruby, or NPM)
//! * List cacheable directories for each of those tools (e.g. build output directories)
//! * Portably bind those cacheable directories to a persistent storage location
//!
//! The core command for all this is `spacectl cache mount`, used in two ways:
//! 1. To detect tools, by running `spacectl cache mount --dry_run=true --detect=*`. This
//!    checks whether each mode supported by `spacectl` applies to the current directory
//! 2. To set up caches, by running `spacectl cache mount --dry_run=false --mode=...`. This
//!    configures *only* the requested cache modes
use std::future::Future;
use std::path::{Path, PathBuf};

use command::r#async::Command;
use instant::Instant;
use serde::Deserialize;

use crate::{
    CachePreparationReport, CacheScope, CacheSetupError, RepoCacheKey, aggregate_mode_stats,
    canonical_modes, failed_invocation,
};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct MountResponse {
    #[serde(default)]
    pub input: MountInput,
    #[serde(default)]
    pub output: MountOutput,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct MountInput {
    #[serde(default)]
    pub modes: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct MountOutput {
    #[serde(default)]
    pub add_envs: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    pub disk_usage: Option<DiskUsage>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Mount {
    #[serde(default)]
    pub mode: String,
    pub cache_hit: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct DiskUsage {
    pub total: String,
    pub used: String,
}

/// Construct a `spacectl` command for detecting all cache modes that apply to
/// `cwd`. Currently, this uses `spacectl cache mount`, though we could use
/// `spacectl cache modes` as well.
fn detect_command(cache_root: &Path, cwd: &Path) -> Command {
    let mut command = Command::new_with_process_group("spacectl");
    command
        .args([
            "cache",
            "mount",
            "--detect=*",
            "--dry_run=true",
            "--cache_root",
        ])
        .arg(cache_root)
        .args(["-o", "json"])
        .current_dir(cwd);
    command
}

/// Construct a `spacectl` command for mounting a specific set of modes on `cwd`.
fn mount_command(cache_root: &Path, cwd: &Path, modes: &[String]) -> Command {
    let mut command = Command::new_with_process_group("spacectl");
    command
        .args(["cache", "mount"])
        .arg(format!("--mode={}", modes.join(",")))
        .args(["--dry_run=false", "--cache_root"])
        .arg(cache_root)
        .args(["-o", "json"])
        .current_dir(cwd);
    command
}

/// Run `spacectl cache mount`.
#[tracing::instrument(
    name = "spacectl_cache_mount",
    skip_all,
    fields(
        tags.cloud_agent = true,
        scope = scope.kind(),
        repo_key = scope.repo_key().map(RepoCacheKey::as_str).unwrap_or(""),
        modes = tracing::field::Empty,
        dry_run,
        relative_cache_dir = %relative_cache_dir.display(),
        duration_ms = tracing::field::Empty,
        disk_usage_total = tracing::field::Empty,
        disk_usage_used = tracing::field::Empty,
    )
)]
pub(super) async fn run_spacectl_mount<F, Fut>(
    scope: CacheScope,
    modes: Vec<String>,
    dry_run: bool,
    relative_cache_dir: PathBuf,
    cache_root: &Path,
    cwd: &Path,
    run_command: &mut F,
) -> CachePreparationReport
where
    F: FnMut(Command) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, CacheSetupError>>,
{
    let command = if dry_run {
        detect_command(cache_root, cwd)
    } else {
        mount_command(cache_root, cwd, &modes)
    };
    let started = Instant::now();
    let result = async {
        let bytes = run_command(command).await?;
        serde_json::from_slice::<MountResponse>(&bytes)
            .map_err(|_| CacheSetupError::JsonParseFailed)
    }
    .await;
    let duration = started.elapsed();
    let span = tracing::Span::current();
    span.record("duration_ms", duration.as_millis() as u64);

    match result {
        Ok(response) => {
            if let Some(disk_usage) = &response.output.disk_usage {
                span.record("disk_usage_total", disk_usage.total.as_str());
                span.record("disk_usage_used", disk_usage.used.as_str());
            }
            let selected_modes = if dry_run {
                canonical_modes(response.input.modes.clone())
            } else {
                modes.clone()
            };
            span.record("modes", selected_modes.join(","));
            let mode_stats = aggregate_mode_stats(&selected_modes, &response);
            for (mode, stats) in &mode_stats {
                tracing::info!(
                    mode,
                    cache_hits = stats.cache_hits,
                    cache_misses = stats.cache_misses,
                    "spacectl cache mode result"
                );
            }
            CachePreparationReport {
                scope,
                modes: selected_modes,
                relative_cache_dir,
                response: Some(response),
                error: None,
                duration,
                mode_stats,
            }
        }
        Err(err) => {
            tracing::error!(error = ?err);
            failed_invocation(scope, modes, relative_cache_dir, err, duration)
        }
    }
}

#[cfg(test)]
#[path = "spacectl_tests.rs"]
mod tests;
