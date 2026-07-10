use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use command::r#async::Command;
use sha2::{Digest as _, Sha256};
use warp_completer::completer::CommandExitStatus;
use warp_isolation_platform::namespace::spacectl::{
    Spacectl, SpacectlCacheMode, SpacectlMount, SpacectlMountResponse,
};
use warp_isolation_platform::IsolationPlatformType;
use warpui::r#async::FutureExt as _;
use warpui::ModelSpawner;

use super::terminal::TerminalDriver;
use crate::ai::agent_sdk::setup_observability::NamespaceCacheMountReport;
use crate::ai::cloud_environments::SourceRepo;
use crate::server::server_api::ai::{
    AgentRunClientCacheInvocationPayload, AgentRunClientCacheModePayload,
};
use crate::terminal::model::session::command_executor::shell_escape_single_quotes;
use crate::terminal::shell::ShellType;

pub(super) const BUILD_CACHE_ROOT_ENV: &str = "WARP_BUILD_CACHE_ROOT";
const SHARED_SCRATCH_CWD: &str = "/tmp/warp-spacectl-shared";
const CACHE_OPERATION_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Eq, PartialEq)]
struct NonEmptyCacheModes(Vec<SpacectlCacheMode>);

impl NonEmptyCacheModes {
    fn new(modes: impl IntoIterator<Item = SpacectlCacheMode>) -> Result<Self, CacheSetupError> {
        let modes = modes.into_iter().collect::<BTreeSet<_>>();
        if modes.is_empty() {
            return Err(CacheSetupError::EmptySharedModes);
        }
        Ok(Self(modes.into_iter().collect()))
    }

    fn as_slice(&self) -> &[SpacectlCacheMode] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CacheMountScope {
    Shared { modes: NonEmptyCacheModes },
    Repository { name: String, key: RepoCacheKey },
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RepoCacheKey(String);

impl RepoCacheKey {
    fn new(value: impl Into<String>) -> Result<Self, CacheSetupError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(CacheSetupError::InvalidRepoCacheKey);
        }
        Ok(Self(value))
    }

    fn for_repo(repo: &SourceRepo) -> Result<Self, CacheSetupError> {
        let forge_host = repo.code_forge.unwrap_or_default().host();
        let identity = format!("{forge_host}/{}/{}", repo.owner, repo.repo);
        Self::new(hex::encode(Sha256::digest(identity.as_bytes())))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CacheMountInvocation {
    scope: CacheMountScope,
    cwd: PathBuf,
    cache_root: PathBuf,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct CacheMountPlan {
    repositories: Vec<CacheMountInvocation>,
    shared_root: PathBuf,
    scratch_cwd: PathBuf,
    setup_is_error: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CachePlatform {
    #[cfg(any(target_os = "linux", test))]
    Linux,
    #[cfg(any(target_os = "macos", test))]
    MacOS,
    #[cfg(any(not(any(target_os = "linux", target_os = "macos")), test))]
    Other,
}

#[derive(Debug, thiserror::Error)]
enum CacheSetupError {
    #[error("cache operation failed: {0}")]
    Operation(String),
    #[error("repository cache key must be a lowercase 64-character SHA-256 digest")]
    InvalidRepoCacheKey,
    #[error("shared cache mode union must not be empty")]
    EmptySharedModes,
    #[error("cache environment variable name is invalid")]
    InvalidEnvironmentName,
}

#[async_trait]
trait CacheCommandAdapter {
    async fn mount_detected_cache(
        &self,
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, CacheSetupError>;

    async fn mount_cache(
        &self,
        modes: &[SpacectlCacheMode],
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, CacheSetupError>;
}

#[derive(Default)]
struct SpacectlCacheCommandAdapter {
    spacectl: Spacectl,
}

#[async_trait]
impl CacheCommandAdapter for SpacectlCacheCommandAdapter {
    async fn mount_detected_cache(
        &self,
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, CacheSetupError> {
        self.spacectl
            .mount_detected_cache(cache_root, cwd)
            .await
            .map_err(|error| CacheSetupError::Operation(error.to_string()))
    }

    async fn mount_cache(
        &self,
        modes: &[SpacectlCacheMode],
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, CacheSetupError> {
        self.spacectl
            .mount_cache(modes, cache_root, cwd)
            .await
            .map_err(|error| CacheSetupError::Operation(error.to_string()))
    }
}

#[async_trait]
trait CacheRuntime {
    fn platform(&self) -> CachePlatform;

    async fn command_exists(&self, command: &str) -> Result<bool, CacheSetupError>;

    async fn create_dir_all(&self, path: &Path) -> Result<(), CacheSetupError>;

    async fn prepare_empty_dir(&self, path: &Path) -> Result<(), CacheSetupError>;
}

struct SystemCacheRuntime;

#[async_trait]
impl CacheRuntime for SystemCacheRuntime {
    fn platform(&self) -> CachePlatform {
        #[cfg(target_os = "linux")]
        {
            CachePlatform::Linux
        }
        #[cfg(target_os = "macos")]
        {
            CachePlatform::MacOS
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            CachePlatform::Other
        }
    }

    async fn command_exists(&self, command: &str) -> Result<bool, CacheSetupError> {
        Ok(Command::new(command)
            .arg("--version")
            .output()
            .await
            .is_ok())
    }

    async fn create_dir_all(&self, path: &Path) -> Result<(), CacheSetupError> {
        async_fs::create_dir_all(path)
            .await
            .map_err(|error| CacheSetupError::Operation(error.to_string()))
    }

    async fn prepare_empty_dir(&self, path: &Path) -> Result<(), CacheSetupError> {
        match async_fs::remove_dir_all(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(CacheSetupError::Operation(error.to_string())),
        }
        self.create_dir_all(path).await
    }
}

#[async_trait]
trait SessionEnvironmentExporter {
    async fn export(&self, environment: &BTreeMap<String, String>) -> Result<(), CacheSetupError>;
}

struct TerminalSessionEnvironmentExporter<'a> {
    spawner: &'a ModelSpawner<TerminalDriver>,
}

#[async_trait]
impl SessionEnvironmentExporter for TerminalSessionEnvironmentExporter<'_> {
    async fn export(&self, environment: &BTreeMap<String, String>) -> Result<(), CacheSetupError> {
        let shell_type = self
            .spawner
            .spawn(|driver, ctx| {
                driver
                    .active_session_shell_type(ctx)
                    .unwrap_or(ShellType::Bash)
            })
            .await
            .map_err(|error| CacheSetupError::Operation(error.to_string()))?;
        let command = build_export_command(environment, shell_type)?;
        let output = self
            .spawner
            .spawn(move |driver, ctx| driver.execute_silent_command(command, ctx))
            .await
            .map_err(|error| CacheSetupError::Operation(error.to_string()))?
            .await
            .map_err(|error| CacheSetupError::Operation(error.to_string()))?;
        if output.status != CommandExitStatus::Success {
            return Err(CacheSetupError::Operation(
                "session environment export command failed".to_owned(),
            ));
        }
        Ok(())
    }
}

fn build_export_command(
    environment: &BTreeMap<String, String>,
    shell_type: ShellType,
) -> Result<String, CacheSetupError> {
    if environment.is_empty() {
        return Ok(":".to_owned());
    }

    environment
        .iter()
        .map(|(name, value)| {
            if !is_valid_environment_name(name) {
                return Err(CacheSetupError::InvalidEnvironmentName);
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CacheModeReport {
    cache_hits: u64,
    cache_misses: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CacheInvocationReport {
    scope: CacheMountScope,
    is_error: bool,
    modes: BTreeMap<String, CacheModeReport>,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct CacheSetupOutcome {
    setup_is_error: bool,
    invocations: Vec<CacheInvocationReport>,
    exported_environment: BTreeMap<String, String>,
}

impl CacheSetupOutcome {
    #[cfg(test)]
    fn is_error(&self) -> bool {
        self.setup_is_error
            || self
                .invocations
                .iter()
                .any(|invocation| invocation.is_error)
    }

    fn into_event_report(self) -> NamespaceCacheMountReport {
        let invocations = self
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
                    CacheMountScope::Shared { .. } => {
                        AgentRunClientCacheInvocationPayload::shared(invocation.is_error, modes)
                    }
                    CacheMountScope::Repository { key, .. } => {
                        AgentRunClientCacheInvocationPayload::repository(
                            key.0,
                            invocation.is_error,
                            modes,
                        )
                    }
                }
            })
            .collect();
        NamespaceCacheMountReport::new(self.setup_is_error, invocations)
    }
}

pub(super) fn build_cache_root(
    platform: Option<IsolationPlatformType>,
    value: Option<OsString>,
) -> Option<PathBuf> {
    if platform != Some(IsolationPlatformType::Namespace) {
        return None;
    }
    let root = PathBuf::from(value?);
    (!root.as_os_str().is_empty() && root.is_absolute()).then_some(root)
}

pub(super) async fn setup_namespace_caches(
    repos: &[SourceRepo],
    working_dir: &Path,
    build_root: &Path,
    spawner: &ModelSpawner<TerminalDriver>,
) -> NamespaceCacheMountReport {
    let adapter = SpacectlCacheCommandAdapter::default();
    let runtime = SystemCacheRuntime;
    let exporter = TerminalSessionEnvironmentExporter { spawner };
    run_cache_setup(
        repos,
        working_dir,
        build_root,
        &adapter,
        &runtime,
        &exporter,
        CACHE_OPERATION_TIMEOUT,
    )
    .await
    .into_event_report()
}

async fn run_cache_setup(
    repos: &[SourceRepo],
    working_dir: &Path,
    build_root: &Path,
    adapter: &impl CacheCommandAdapter,
    runtime: &impl CacheRuntime,
    exporter: &impl SessionEnvironmentExporter,
    timeout: Duration,
) -> CacheSetupOutcome {
    let plan = build_mount_plan(repos, working_dir, build_root);
    execute_mount_plan(plan, adapter, runtime, exporter, timeout).await
}

fn build_mount_plan(repos: &[SourceRepo], working_dir: &Path, build_root: &Path) -> CacheMountPlan {
    let mut plan = CacheMountPlan {
        shared_root: build_root.join("shared"),
        scratch_cwd: PathBuf::from(SHARED_SCRATCH_CWD),
        ..Default::default()
    };
    for repo in repos {
        match RepoCacheKey::for_repo(repo) {
            Ok(key) => plan.repositories.push(CacheMountInvocation {
                scope: CacheMountScope::Repository {
                    name: format!("{}/{}", repo.owner, repo.repo),
                    key: key.clone(),
                },
                cwd: working_dir.join(&repo.repo),
                cache_root: build_root.join("repos").join(key.as_str()),
            }),
            Err(error) => {
                log::warn!("Failed to derive Namespace cache repository key: {error}");
                plan.setup_is_error = true;
            }
        }
    }
    plan.repositories.sort_by(|left, right| {
        let CacheMountScope::Repository { key: left_key, .. } = &left.scope else {
            unreachable!("repository plan contains a shared invocation");
        };
        let CacheMountScope::Repository { key: right_key, .. } = &right.scope else {
            unreachable!("repository plan contains a shared invocation");
        };
        left_key.cmp(right_key)
    });
    plan
}

async fn execute_mount_plan(
    plan: CacheMountPlan,
    adapter: &impl CacheCommandAdapter,
    runtime: &impl CacheRuntime,
    exporter: &impl SessionEnvironmentExporter,
    timeout: Duration,
) -> CacheSetupOutcome {
    let mut setup_is_error = plan.setup_is_error;
    let mut reports = Vec::with_capacity(plan.repositories.len() + 1);
    let mut repo_environment = BTreeMap::new();
    let mut environment_owners = BTreeMap::<String, String>::new();
    let mut shared_modes = BTreeSet::new();

    for invocation in plan.repositories {
        let CacheMountScope::Repository { name, key } = &invocation.scope else {
            unreachable!("repository plan contains a shared invocation");
        };
        if let Err(error) = runtime.create_dir_all(&invocation.cache_root).await {
            log::warn!("Failed to create Namespace repository cache root: {error}");
            reports.push(failed_invocation_report(invocation.scope));
            continue;
        }

        match adapter
            .mount_detected_cache(&invocation.cache_root, &invocation.cwd)
            .with_timeout(timeout)
            .await
        {
            Ok(Ok(response)) => {
                shared_modes.extend(response.input_modes.iter().cloned());
                overlay_repository_environment(
                    &mut repo_environment,
                    &mut environment_owners,
                    key.as_str(),
                    &response.add_envs,
                );
                reports.push(successful_invocation_report(
                    invocation.scope,
                    response.input_modes,
                    response.mounts,
                    None,
                ));
            }
            Ok(Err(error)) => {
                log::warn!("Failed to mount Namespace caches for {name}: {error}");
                reports.push(failed_invocation_report(invocation.scope));
            }
            Err(_) => {
                log::warn!("Timed out mounting Namespace caches for {name}");
                reports.push(failed_invocation_report(invocation.scope));
            }
        }
    }

    let (platform_mode, platform_error) = detect_platform_mode(runtime, timeout).await;
    setup_is_error |= platform_error;
    if let Some(mode) = platform_mode {
        shared_modes.insert(mode);
    }

    let mut selected_environment = repo_environment;
    if let Ok(modes) = NonEmptyCacheModes::new(shared_modes) {
        let scope = CacheMountScope::Shared {
            modes: modes.clone(),
        };
        let root_result = runtime.create_dir_all(&plan.shared_root).await;
        let scratch_result = runtime.prepare_empty_dir(&plan.scratch_cwd).await;
        match (root_result, scratch_result) {
            (Ok(()), Ok(())) => {
                match adapter
                    .mount_cache(modes.as_slice(), &plan.shared_root, &plan.scratch_cwd)
                    .with_timeout(timeout)
                    .await
                {
                    Ok(Ok(response)) => {
                        selected_environment = response.add_envs;
                        reports.push(successful_invocation_report(
                            scope,
                            response.input_modes,
                            response.mounts,
                            Some(&modes),
                        ));
                    }
                    Ok(Err(error)) => {
                        log::warn!("Failed to mount shared Namespace caches: {error}");
                        reports.push(failed_shared_invocation_report(scope, &modes));
                    }
                    Err(_) => {
                        log::warn!("Timed out mounting shared Namespace caches");
                        reports.push(failed_shared_invocation_report(scope, &modes));
                    }
                }
            }
            (root_result, scratch_result) => {
                if let Err(error) = root_result {
                    log::warn!("Failed to create shared Namespace cache root: {error}");
                }
                if let Err(error) = scratch_result {
                    log::warn!("Failed to prepare Namespace cache scratch directory: {error}");
                }
                reports.push(failed_shared_invocation_report(scope, &modes));
            }
        }
    }

    if let Err(error) = exporter.export(&selected_environment).await {
        log::warn!("Failed to export Namespace cache environment: {error}");
        setup_is_error = true;
    }

    CacheSetupOutcome {
        setup_is_error,
        invocations: reports,
        exported_environment: selected_environment,
    }
}

fn overlay_repository_environment(
    aggregate: &mut BTreeMap<String, String>,
    owners: &mut BTreeMap<String, String>,
    repo_key: &str,
    contribution: &BTreeMap<String, String>,
) {
    for (name, value) in contribution {
        if let Some(previous_owner) = owners.get(name) {
            if aggregate.get(name) != Some(value) {
                log::warn!(
                    "Namespace cache repositories {previous_owner} and {repo_key} emitted conflicting values for {name}; using {repo_key}"
                );
            }
        }
        owners.insert(name.clone(), repo_key.to_owned());
        aggregate.insert(name.clone(), value.clone());
    }
}

fn successful_invocation_report(
    scope: CacheMountScope,
    input_modes: Vec<SpacectlCacheMode>,
    mounts: Vec<SpacectlMount>,
    expected_modes: Option<&NonEmptyCacheModes>,
) -> CacheInvocationReport {
    let mut modes = expected_modes
        .into_iter()
        .flat_map(|modes| modes.as_slice())
        .chain(input_modes.iter())
        .map(|mode| (mode.as_str().to_owned(), CacheModeReport::default()))
        .collect::<BTreeMap<_, _>>();
    aggregate_mounts(&mut modes, mounts);
    CacheInvocationReport {
        scope,
        is_error: false,
        modes,
    }
}

fn failed_invocation_report(scope: CacheMountScope) -> CacheInvocationReport {
    CacheInvocationReport {
        scope,
        is_error: true,
        modes: BTreeMap::new(),
    }
}

fn failed_shared_invocation_report(
    scope: CacheMountScope,
    modes: &NonEmptyCacheModes,
) -> CacheInvocationReport {
    CacheInvocationReport {
        scope,
        is_error: true,
        modes: modes
            .as_slice()
            .iter()
            .map(|mode| (mode.as_str().to_owned(), CacheModeReport::default()))
            .collect(),
    }
}

fn aggregate_mounts(
    mode_reports: &mut BTreeMap<String, CacheModeReport>,
    mounts: Vec<SpacectlMount>,
) {
    for mount in mounts {
        let report = mode_reports
            .entry(mount.mode.as_str().to_owned())
            .or_default();
        if mount.cache_hit {
            report.cache_hits += 1;
        } else {
            report.cache_misses += 1;
        }
    }
}

async fn detect_platform_mode(
    runtime: &impl CacheRuntime,
    timeout: Duration,
) -> (Option<SpacectlCacheMode>, bool) {
    let (command, mode) = match runtime.platform() {
        #[cfg(any(target_os = "linux", test))]
        CachePlatform::Linux => ("apt-config", "apt"),
        #[cfg(any(target_os = "macos", test))]
        CachePlatform::MacOS => ("brew", "brew"),
        #[cfg(any(not(any(target_os = "linux", target_os = "macos")), test))]
        CachePlatform::Other => return (None, false),
    };
    match runtime.command_exists(command).with_timeout(timeout).await {
        Ok(Ok(true)) => match SpacectlCacheMode::new(mode) {
            Ok(mode) => (Some(mode), false),
            Err(error) => {
                log::warn!("Failed to create Namespace platform cache mode: {error}");
                (None, true)
            }
        },
        Ok(Ok(false)) => (None, false),
        Ok(Err(error)) => {
            log::warn!("Failed to detect Namespace platform cache mode: {error}");
            (None, true)
        }
        Err(_) => {
            log::warn!("Timed out detecting Namespace platform cache mode");
            (None, true)
        }
    }
}

#[cfg(test)]
#[path = "namespace_cache_tests.rs"]
mod tests;
