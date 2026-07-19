use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_io::Timer;
use async_trait::async_trait;
use futures_lite::future;
use sha2::{Digest as _, Sha256};
use spacectl::{Spacectl, SpacectlCacheMode, SpacectlMount, SpacectlMountResponse};
use vec1::Vec1;

mod spacectl;

/// Environment variable set by warp-server to identify the mounted build-cache volume.
pub const BUILD_CACHE_ROOT_ENV: &str = "WARP_BUILD_CACHE_ROOT";

const SHARED_SCRATCH_CWD: &str = "/tmp/warp-spacectl-shared";
const CACHE_OPERATION_TIMEOUT: Duration = Duration::from_secs(60);

/// A cloned repository that can contribute detected build-cache modes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Repository {
    forge_host: String,
    owner: String,
    name: String,
    checkout_path: PathBuf,
}

impl Repository {
    /// Creates a repository from its canonical source identity and checkout path.
    pub fn new(
        forge_host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
        checkout_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            forge_host: forge_host.into(),
            owner: owner.into(),
            name: name.into(),
            checkout_path: checkout_path.into(),
        }
    }
}

/// A platform package-manager mode that should be included in the shared cache replay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformCacheMode {
    /// Linux apt package cache.
    Apt,
    /// macOS Homebrew package cache.
    Brew,
}

impl PlatformCacheMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Apt => "apt",
            Self::Brew => "brew",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CacheMountScope {
    Shared { modes: Vec1<SpacectlCacheMode> },
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

    fn for_repo(repo: &Repository) -> Result<Self, CacheSetupError> {
        let identity = format!("{}/{}/{}", repo.forge_host, repo.owner, repo.name);
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

#[derive(Debug, thiserror::Error)]
enum CacheSetupError {
    #[error("cache operation failed: {0}")]
    Operation(String),
    #[error("repository cache key must be a lowercase 64-character SHA-256 digest")]
    InvalidRepoCacheKey,
}

// This boundary deliberately mirrors the two pinned spacectl operations used by the executor.
// It keeps process transport separate from planning and lets tests provide deterministic
// responses; it is not intended to be a provider-agnostic cache abstraction.
#[async_trait]
trait SpacectlClient {
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

#[async_trait]
impl SpacectlClient for Spacectl {
    async fn mount_detected_cache(
        &self,
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, CacheSetupError> {
        Spacectl::mount_detected_cache(self, cache_root, cwd)
            .await
            .map_err(|error| CacheSetupError::Operation(error.to_string()))
    }

    async fn mount_cache(
        &self,
        modes: &[SpacectlCacheMode],
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, CacheSetupError> {
        Spacectl::mount_cache(self, modes, cache_root, cwd)
            .await
            .map_err(|error| CacheSetupError::Operation(error.to_string()))
    }
}

#[async_trait]
trait CacheFileSystem {
    async fn create_dir_all(&self, path: &Path) -> Result<(), CacheSetupError>;

    async fn prepare_empty_dir(&self, path: &Path) -> Result<(), CacheSetupError>;
}

struct SystemCacheFileSystem;

#[async_trait]
impl CacheFileSystem for SystemCacheFileSystem {
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

/// Aggregate cache-hit details for one mode in one spacectl invocation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CacheModeReport {
    /// Number of mounted paths that already existed in the cache.
    pub cache_hits: u64,
    /// Number of mounted paths newly created in the cache.
    pub cache_misses: u64,
}

/// Public diagnostic scope for one logical cache invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheInvocationScope {
    /// The final shared replay.
    Shared,
    /// A repository-scoped detection and mount.
    Repository {
        /// Stable opaque SHA-256 source identity.
        repo_key: String,
    },
}

/// Diagnostic result for one logical cache invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheInvocationReport {
    /// Shared or repository scope.
    pub scope: CacheInvocationScope,
    /// Whether this invocation failed.
    pub is_error: bool,
    /// Per-mode hit and miss counts.
    pub modes: BTreeMap<String, CacheModeReport>,
}

/// Result of build-cache planning and spacectl execution.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct CacheSetupOutcome {
    /// Whether setup failed outside a specific spacectl invocation.
    pub setup_is_error: bool,
    /// Ordered invocation diagnostics.
    pub invocations: Vec<CacheInvocationReport>,
    /// Environment variables selected for the live agent session.
    pub environment: BTreeMap<String, String>,
}

impl CacheSetupOutcome {
    /// Returns whether setup or any individual invocation failed.
    pub fn is_error(&self) -> bool {
        self.setup_is_error
            || self
                .invocations
                .iter()
                .any(|invocation| invocation.is_error)
    }

    /// Marks a caller-owned integration step, such as session export, as failed.
    pub fn mark_error(&mut self) {
        self.setup_is_error = true;
    }
}

/// Detects and mounts repository caches, then replays the successful mode union under shared root.
pub async fn setup(
    repositories: &[Repository],
    build_root: &Path,
    platform_mode: Option<PlatformCacheMode>,
) -> CacheSetupOutcome {
    run_cache_setup(
        repositories,
        build_root,
        platform_mode,
        &Spacectl::default(),
        &SystemCacheFileSystem,
        CACHE_OPERATION_TIMEOUT,
    )
    .await
}

async fn run_cache_setup(
    repositories: &[Repository],
    build_root: &Path,
    platform_mode: Option<PlatformCacheMode>,
    spacectl: &impl SpacectlClient,
    file_system: &impl CacheFileSystem,
    timeout: Duration,
) -> CacheSetupOutcome {
    let plan = build_mount_plan(repositories, build_root);
    execute_mount_plan(plan, platform_mode, spacectl, file_system, timeout).await
}

fn build_mount_plan(repositories: &[Repository], build_root: &Path) -> CacheMountPlan {
    let mut plan = CacheMountPlan {
        shared_root: build_root.join("shared"),
        scratch_cwd: PathBuf::from(SHARED_SCRATCH_CWD),
        ..Default::default()
    };
    for repository in repositories {
        match RepoCacheKey::for_repo(repository) {
            Ok(key) => plan.repositories.push(CacheMountInvocation {
                scope: CacheMountScope::Repository {
                    name: format!("{}/{}", repository.owner, repository.name),
                    key: key.clone(),
                },
                cwd: repository.checkout_path.clone(),
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
    platform_mode: Option<PlatformCacheMode>,
    spacectl: &impl SpacectlClient,
    file_system: &impl CacheFileSystem,
    timeout: Duration,
) -> CacheSetupOutcome {
    let setup_is_error = plan.setup_is_error;
    let mut reports = Vec::with_capacity(plan.repositories.len() + 1);
    let mut repository_environment = BTreeMap::new();
    let mut environment_owners = BTreeMap::<String, String>::new();
    let mut shared_modes = BTreeSet::new();

    for invocation in plan.repositories {
        let CacheMountScope::Repository { name, key } = &invocation.scope else {
            unreachable!("repository plan contains a shared invocation");
        };
        if let Err(error) = file_system.create_dir_all(&invocation.cache_root).await {
            log::warn!("Failed to create Namespace repository cache root: {error}");
            reports.push(failed_invocation_report(invocation.scope));
            continue;
        }

        match with_timeout(
            spacectl.mount_detected_cache(&invocation.cache_root, &invocation.cwd),
            timeout,
        )
        .await
        {
            Some(Ok(response)) => {
                shared_modes.extend(response.input_modes.iter().cloned());
                overlay_repository_environment(
                    &mut repository_environment,
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
            Some(Err(error)) => {
                log::warn!("Failed to mount Namespace caches for {name}: {error}");
                reports.push(failed_invocation_report(invocation.scope));
            }
            None => {
                log::warn!("Timed out mounting Namespace caches for {name}");
                reports.push(failed_invocation_report(invocation.scope));
            }
        }
    }

    if let Some(platform_mode) = platform_mode {
        match SpacectlCacheMode::new(platform_mode.as_str()) {
            Ok(mode) => {
                shared_modes.insert(mode);
            }
            Err(error) => {
                log::warn!("Failed to create Namespace platform cache mode: {error}");
            }
        }
    }

    let mut selected_environment = repository_environment;
    if let Ok(modes) = Vec1::try_from_vec(shared_modes.into_iter().collect()) {
        let scope = CacheMountScope::Shared {
            modes: modes.clone(),
        };
        let root_result = file_system.create_dir_all(&plan.shared_root).await;
        let scratch_result = file_system.prepare_empty_dir(&plan.scratch_cwd).await;
        match (root_result, scratch_result) {
            (Ok(()), Ok(())) => {
                match with_timeout(
                    spacectl.mount_cache(&modes, &plan.shared_root, &plan.scratch_cwd),
                    timeout,
                )
                .await
                {
                    Some(Ok(response)) => {
                        selected_environment = response.add_envs;
                        reports.push(successful_invocation_report(
                            scope,
                            response.input_modes,
                            response.mounts,
                            Some(&modes),
                        ));
                    }
                    Some(Err(error)) => {
                        log::warn!("Failed to mount shared Namespace caches: {error}");
                        reports.push(failed_shared_invocation_report(scope, &modes));
                    }
                    None => {
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

    CacheSetupOutcome {
        setup_is_error,
        invocations: reports,
        environment: selected_environment,
    }
}

async fn with_timeout<F: Future>(operation: F, timeout: Duration) -> Option<F::Output> {
    future::race(async move { Some(operation.await) }, async move {
        Timer::after(timeout).await;
        None
    })
    .await
}

fn overlay_repository_environment(
    aggregate: &mut BTreeMap<String, String>,
    owners: &mut BTreeMap<String, String>,
    repo_key: &str,
    contribution: &BTreeMap<String, String>,
) {
    for (name, value) in contribution {
        if let Some(previous_owner) = owners.get(name)
            && aggregate.get(name) != Some(value)
        {
            log::warn!(
                "Namespace cache repositories {previous_owner} and {repo_key} emitted conflicting values for {name}; using {repo_key}"
            );
        }
        owners.insert(name.clone(), repo_key.to_owned());
        aggregate.insert(name.clone(), value.clone());
    }
}

fn successful_invocation_report(
    scope: CacheMountScope,
    input_modes: Vec<SpacectlCacheMode>,
    mounts: Vec<SpacectlMount>,
    expected_modes: Option<&Vec1<SpacectlCacheMode>>,
) -> CacheInvocationReport {
    let mut modes = expected_modes
        .into_iter()
        .flat_map(|modes| modes.iter())
        .chain(input_modes.iter())
        .map(|mode| (mode.as_str().to_owned(), CacheModeReport::default()))
        .collect::<BTreeMap<_, _>>();
    aggregate_mounts(&mut modes, mounts);
    CacheInvocationReport {
        scope: report_scope(scope),
        is_error: false,
        modes,
    }
}

fn failed_invocation_report(scope: CacheMountScope) -> CacheInvocationReport {
    CacheInvocationReport {
        scope: report_scope(scope),
        is_error: true,
        modes: BTreeMap::new(),
    }
}

fn failed_shared_invocation_report(
    scope: CacheMountScope,
    modes: &Vec1<SpacectlCacheMode>,
) -> CacheInvocationReport {
    CacheInvocationReport {
        scope: report_scope(scope),
        is_error: true,
        modes: modes
            .iter()
            .map(|mode| (mode.as_str().to_owned(), CacheModeReport::default()))
            .collect(),
    }
}

fn report_scope(scope: CacheMountScope) -> CacheInvocationScope {
    match scope {
        CacheMountScope::Shared { .. } => CacheInvocationScope::Shared,
        CacheMountScope::Repository { key, .. } => {
            CacheInvocationScope::Repository { repo_key: key.0 }
        }
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
