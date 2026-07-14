// The injected executor must receive an unexecuted std command so tests can inspect exact argv/cwd.
#![allow(clippy::disallowed_types)]
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tracing_futures::Instrument as _;
use warp_errors::{ErrorExt, register_error};

pub mod spacectl;

use spacectl::{MountResponse, detect_command, mount_command, parse_mount_response};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RepoIdentity {
    pub forge_host: String,
    pub owner: String,
    pub repo: String,
}

impl RepoIdentity {
    pub fn new(
        forge_host: impl Into<String>,
        owner: impl Into<String>,
        repo: impl Into<String>,
    ) -> Self {
        Self {
            forge_host: normalize_identity_part(forge_host.into()),
            owner: normalize_identity_part(owner.into()),
            repo: normalize_identity_part(repo.into()),
        }
    }
}

fn normalize_identity_part(value: String) -> String {
    value.trim().to_ascii_lowercase()
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RepoCacheKey(String);

impl RepoCacheKey {
    pub fn derive(identity: &RepoIdentity) -> Self {
        let mut hasher = Sha256::new();
        for part in [&identity.forge_host, &identity.owner, &identity.repo] {
            let bytes = part.as_bytes();
            hasher.update((bytes.len() as u64).to_be_bytes());
            hasher.update(bytes);
        }
        Self(hex::encode(hasher.finalize()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoCacheKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RepoCacheKey {
    type Error = InvalidRepoCacheKey;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(InvalidRepoCacheKey);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("repository cache keys must be exactly 64 lowercase hexadecimal characters")]
pub struct InvalidRepoCacheKey;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheScope {
    Repository { name: String, key: RepoCacheKey },
    Global,
}

impl CacheScope {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Repository { .. } => "repository",
            Self::Global => "global",
        }
    }

    pub fn repo_key(&self) -> Option<&RepoCacheKey> {
        match self {
            Self::Repository { key, .. } => Some(key),
            Self::Global => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryCacheSource {
    pub name: String,
    pub identity: RepoIdentity,
    pub cwd: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheConfiguration {
    pub scope: CacheScope,
    pub cwd: PathBuf,
    pub relative_cache_dir: PathBuf,
    pub modes: Vec<String>,
}

/// An executable plan contains zero or more repository configurations in ascending
/// [`RepoCacheKey`] order, followed by exactly one global configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheSetupPlan {
    pub cache_root: PathBuf,
    pub configurations: Vec<CacheConfiguration>,
}

impl CacheSetupPlan {
    pub fn try_new(
        cache_root: PathBuf,
        configurations: Vec<CacheConfiguration>,
    ) -> Result<Self, PlanInvariantError> {
        let plan = Self {
            cache_root,
            configurations,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), PlanInvariantError> {
        let Some((global, repositories)) = self.configurations.split_last() else {
            return Err(PlanInvariantError);
        };
        if !matches!(global.scope, CacheScope::Global) {
            return Err(PlanInvariantError);
        }

        let mut previous_key: Option<&RepoCacheKey> = None;
        for configuration in repositories {
            let CacheScope::Repository { key, .. } = &configuration.scope else {
                return Err(PlanInvariantError);
            };
            if previous_key.is_some_and(|previous| previous > key) {
                return Err(PlanInvariantError);
            }
            previous_key = Some(key);
        }

        for configuration in &self.configurations {
            if configuration.modes.is_empty()
                || !is_sorted_deduplicated(&configuration.modes)
                || !is_safe_relative_cache_dir(&configuration.relative_cache_dir)
            {
                return Err(PlanInvariantError);
            }
        }
        if global.relative_cache_dir != Path::new("shared") {
            return Err(PlanInvariantError);
        }
        Ok(())
    }
}

fn is_sorted_deduplicated(values: &[String]) -> bool {
    values.windows(2).all(|window| window[0] < window[1])
}

fn is_safe_relative_cache_dir(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("cache setup plan invariant violated")]
pub struct PlanInvariantError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostPlatform {
    Linux,
    MacOs,
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemCacheTools {
    pub apt_config: bool,
    pub brew: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CacheSetupError {
    #[error("failed to create a build cache directory")]
    RootCreationFailed,
    #[error("failed to spawn spacectl")]
    SpawnFailed,
    #[error("spacectl exited unsuccessfully")]
    NonzeroExit { exit_code: Option<i32> },
    #[error("failed to parse spacectl JSON output")]
    JsonParseFailed,
    #[error("spacectl timed out")]
    Timeout,
    #[error("failed to export build cache environment variables")]
    EnvExportFailed,
}

impl CacheSetupError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::RootCreationFailed => "root_creation_failed",
            Self::SpawnFailed => "spawn_failed",
            Self::NonzeroExit { .. } => "nonzero_exit",
            Self::JsonParseFailed => "json_parse_failed",
            Self::Timeout => "timeout",
            Self::EnvExportFailed => "env_export_failed",
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            Self::NonzeroExit { exit_code } => *exit_code,
            Self::RootCreationFailed
            | Self::SpawnFailed
            | Self::JsonParseFailed
            | Self::Timeout
            | Self::EnvExportFailed => None,
        }
    }
}

impl ErrorExt for CacheSetupError {
    fn is_actionable(&self) -> bool {
        match self {
            Self::JsonParseFailed | Self::EnvExportFailed => true,
            Self::RootCreationFailed
            | Self::SpawnFailed
            | Self::NonzeroExit { .. }
            | Self::Timeout => false,
        }
    }
}
register_error!(CacheSetupError);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModeCacheStats {
    pub cache_hits: usize,
    pub cache_misses: usize,
}

#[derive(Clone, Debug)]
pub struct InvocationReport {
    pub scope: CacheScope,
    pub modes: Vec<String>,
    pub dry_run: bool,
    pub relative_cache_dir: PathBuf,
    pub response: Option<MountResponse>,
    pub error: Option<CacheSetupError>,
    pub duration: Duration,
    pub mode_stats: BTreeMap<String, ModeCacheStats>,
}

impl InvocationReport {
    pub fn succeeded(&self) -> bool {
        self.error.is_none() && self.response.is_some()
    }
}

#[derive(Clone, Debug, Default)]
pub struct CacheSetupReport {
    pub plan: Option<CacheSetupPlan>,
    pub invocations: Vec<InvocationReport>,
    pub add_envs: BTreeMap<String, String>,
    pub export_script: Option<String>,
}

impl CacheSetupReport {
    pub fn degradations(&self) -> impl Iterator<Item = &InvocationReport> {
        self.invocations
            .iter()
            .filter(|invocation| invocation.error.is_some())
    }
}

#[derive(Clone)]
struct Detection {
    source: RepositoryCacheSource,
    key: RepoCacheKey,
    modes: Vec<String>,
}

pub async fn setup_cache<F, Fut>(
    cache_root: PathBuf,
    repositories: Vec<RepositoryCacheSource>,
    platform: HostPlatform,
    tools: SystemCacheTools,
    run_command: F,
) -> CacheSetupReport
where
    F: FnMut(Command) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, CacheSetupError>>,
{
    let span = tracing::info_span!("setup_caches", tags.cloud_agent = true);
    setup_cache_inner(cache_root, repositories, platform, tools, run_command)
        .instrument(span)
        .await
}

async fn setup_cache_inner<F, Fut>(
    cache_root: PathBuf,
    repositories: Vec<RepositoryCacheSource>,
    platform: HostPlatform,
    tools: SystemCacheTools,
    mut run_command: F,
) -> CacheSetupReport
where
    F: FnMut(Command) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, CacheSetupError>>,
{
    let mut report = CacheSetupReport::default();
    let mut keyed_repositories: Vec<_> = repositories
        .into_iter()
        .map(|source| {
            let key = RepoCacheKey::derive(&source.identity);
            (key, source)
        })
        .collect();
    keyed_repositories.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut detections = Vec::new();
    for (key, source) in keyed_repositories {
        let relative_cache_dir = PathBuf::from("repos").join(key.as_str());
        let configuration_root = cache_root.join(&relative_cache_dir);
        let scope = CacheScope::Repository {
            name: source.name.clone(),
            key: key.clone(),
        };
        if std::fs::create_dir_all(&configuration_root).is_err() {
            report.invocations.push(failed_invocation(
                scope,
                Vec::new(),
                true,
                relative_cache_dir,
                CacheSetupError::RootCreationFailed,
                Duration::ZERO,
            ));
            continue;
        }

        let command = detect_command(&configuration_root, &source.cwd);
        let invocation = invoke(
            scope,
            Vec::new(),
            true,
            relative_cache_dir,
            command,
            &mut run_command,
        )
        .await;
        if let Some(response) = &invocation.response {
            let modes = canonical_modes(response.input.modes.clone());
            if !modes.is_empty() {
                detections.push(Detection { source, key, modes });
            }
        }
        report.invocations.push(invocation);
    }

    let plan = match construct_plan(cache_root, detections, platform, tools) {
        Ok(Some(plan)) => plan,
        Ok(None) => return report,
        Err(error) => {
            report.invocations.push(failed_invocation(
                CacheScope::Global,
                Vec::new(),
                false,
                PathBuf::from("shared"),
                error,
                Duration::ZERO,
            ));
            return report;
        }
    };

    let mut repository_env = BTreeMap::new();
    let mut global_env = None;
    for configuration in &plan.configurations {
        let configuration_root = plan.cache_root.join(&configuration.relative_cache_dir);
        let invocation = if std::fs::create_dir_all(&configuration_root).is_err() {
            failed_invocation(
                configuration.scope.clone(),
                configuration.modes.clone(),
                false,
                configuration.relative_cache_dir.clone(),
                CacheSetupError::RootCreationFailed,
                Duration::ZERO,
            )
        } else {
            let command = mount_command(
                &configuration_root,
                &configuration.cwd,
                &configuration.modes,
            );
            invoke(
                configuration.scope.clone(),
                configuration.modes.clone(),
                false,
                configuration.relative_cache_dir.clone(),
                command,
                &mut run_command,
            )
            .await
        };

        if let Some(response) = &invocation.response {
            match &configuration.scope {
                CacheScope::Repository { .. } => {
                    for (name, value) in &response.output.add_envs {
                        if repository_env.insert(name.clone(), value.clone()).is_some() {
                            log_env_conflict();
                        }
                    }
                }
                CacheScope::Global => {
                    global_env = Some(response.output.add_envs.clone());
                }
            }
        }
        report.invocations.push(invocation);
    }

    report.add_envs = global_env.unwrap_or(repository_env);
    report.export_script = build_export_script(&report.add_envs);
    report.plan = Some(plan);
    report
}

fn log_env_conflict() {
    tracing::warn!(
        target: "build_cache",
        "repository build-cache environment conflict resolved by canonical repository order"
    );
}

fn construct_plan(
    cache_root: PathBuf,
    mut detections: Vec<Detection>,
    platform: HostPlatform,
    tools: SystemCacheTools,
) -> Result<Option<CacheSetupPlan>, CacheSetupError> {
    for detection in &mut detections {
        detection.modes = canonical_modes(std::mem::take(&mut detection.modes));
    }
    let mut global_modes = BTreeSet::new();
    for detection in &detections {
        global_modes.extend(detection.modes.iter().cloned());
    }
    match platform {
        HostPlatform::Linux if tools.apt_config => {
            global_modes.insert("apt".to_owned());
        }
        HostPlatform::MacOs if tools.brew => {
            global_modes.insert("brew".to_owned());
        }
        HostPlatform::Linux | HostPlatform::MacOs | HostPlatform::Other => {}
    }
    if global_modes.is_empty() {
        return Ok(None);
    }

    let scratch = create_retained_scratch_directory(
        detections
            .iter()
            .map(|detection| detection.source.cwd.as_path()),
    )?;
    let mut configurations = detections
        .into_iter()
        .map(|detection| CacheConfiguration {
            scope: CacheScope::Repository {
                name: detection.source.name,
                key: detection.key.clone(),
            },
            cwd: detection.source.cwd,
            relative_cache_dir: PathBuf::from("repos").join(detection.key.as_str()),
            modes: detection.modes,
        })
        .collect::<Vec<_>>();
    configurations.sort_by(|left, right| {
        left.scope
            .repo_key()
            .expect("repository configuration")
            .cmp(right.scope.repo_key().expect("repository configuration"))
    });
    configurations.push(CacheConfiguration {
        scope: CacheScope::Global,
        cwd: scratch,
        relative_cache_dir: PathBuf::from("shared"),
        modes: global_modes.into_iter().collect(),
    });
    CacheSetupPlan::try_new(cache_root, configurations)
        .map(Some)
        .map_err(|_| CacheSetupError::RootCreationFailed)
}

fn create_retained_scratch_directory<'a>(
    repository_paths: impl Iterator<Item = &'a Path>,
) -> Result<PathBuf, CacheSetupError> {
    let repository_paths = repository_paths.collect::<Vec<_>>();
    let directory = tempfile::Builder::new()
        .prefix("warp-spacectl-")
        .tempdir()
        .map_err(|_| CacheSetupError::RootCreationFailed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(|_| CacheSetupError::RootCreationFailed)?;
    }
    if repository_paths
        .iter()
        .any(|repository| directory.path().starts_with(repository))
    {
        return Err(CacheSetupError::RootCreationFailed);
    }
    Ok(directory.keep())
}

async fn invoke<F, Fut>(
    scope: CacheScope,
    modes: Vec<String>,
    dry_run: bool,
    relative_cache_dir: PathBuf,
    command: Command,
    run_command: &mut F,
) -> InvocationReport
where
    F: FnMut(Command) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, CacheSetupError>>,
{
    let started = Instant::now();
    let scope_name = scope.kind();
    let repo_key = scope.repo_key().map(RepoCacheKey::as_str).unwrap_or("");
    let relative_cache_dir_field = relative_cache_dir.to_string_lossy().to_string();
    let span = tracing::info_span!(
        "spacectl_cache_mount",
        tags.cloud_agent = true,
        scope = scope_name,
        repo_key,
        modes = tracing::field::Empty,
        dry_run,
        relative_cache_dir = relative_cache_dir_field,
        error_kind = tracing::field::Empty,
        duration_ms = tracing::field::Empty,
        disk_usage_total = tracing::field::Empty,
        disk_usage_used = tracing::field::Empty,
    );
    let result = async {
        let bytes = run_command(command).await?;
        parse_mount_response(&bytes)
    }
    .instrument(span.clone())
    .await;
    let duration = started.elapsed();
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
                    parent: &span,
                    mode,
                    cache_hits = stats.cache_hits,
                    cache_misses = stats.cache_misses,
                    "spacectl cache mode result"
                );
            }
            InvocationReport {
                scope,
                modes: selected_modes,
                dry_run,
                relative_cache_dir,
                response: Some(response),
                error: None,
                duration,
                mode_stats,
            }
        }
        Err(error) => {
            span.record("error_kind", error.kind());
            failed_invocation(scope, modes, dry_run, relative_cache_dir, error, duration)
        }
    }
}

fn failed_invocation(
    scope: CacheScope,
    modes: Vec<String>,
    dry_run: bool,
    relative_cache_dir: PathBuf,
    error: CacheSetupError,
    duration: Duration,
) -> InvocationReport {
    InvocationReport {
        scope,
        modes,
        dry_run,
        relative_cache_dir,
        response: None,
        error: Some(error),
        duration,
        mode_stats: BTreeMap::new(),
    }
}

fn canonical_modes(modes: Vec<String>) -> Vec<String> {
    modes
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn aggregate_mode_stats(
    selected_modes: &[String],
    response: &MountResponse,
) -> BTreeMap<String, ModeCacheStats> {
    let mut stats = selected_modes
        .iter()
        .cloned()
        .map(|mode| (mode, ModeCacheStats::default()))
        .collect::<BTreeMap<_, _>>();
    for mount in &response.output.mounts {
        let entry = stats.entry(mount.mode.clone()).or_default();
        if mount.cache_hit {
            entry.cache_hits += 1;
        } else {
            entry.cache_misses += 1;
        }
    }
    stats
}

pub fn is_valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub fn posix_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn build_export_script(environment: &BTreeMap<String, String>) -> Option<String> {
    let exports = environment
        .iter()
        .filter_map(|(name, value)| {
            if is_valid_env_name(name) {
                Some(format!("export {name}={}", posix_single_quote(value)))
            } else {
                tracing::warn!(
                    target: "build_cache",
                    "ignored invalid build-cache environment variable name"
                );
                None
            }
        })
        .collect::<Vec<_>>();
    (!exports.is_empty()).then(|| exports.join("; "))
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
