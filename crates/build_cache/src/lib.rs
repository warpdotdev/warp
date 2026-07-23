//! Persistent build cache management for sandboxed agents.
//!
//! This crate configures a sandbox environment to use attached persistent storage for caching
//! build artifacts and downloaded dependencies.
//!
//! The cache setup is aware of specific repositories and tech stacks. It:
//! 1. Scans repositories to identify the tools in use, such as the Swift Package Manager, Maven, Go, and more.
//! 2. Checks for system-level package managers such as Homebrew and `apt-get`.
//! 3. Consolidates the results into a single caching plan.
//! 4. Applies the plan to the system. The result is:
//!    * Per-repo cache mounts, such as `./target` in Rust codebases
//!    * Global cache mounts, such as the Go module download cache
//!    * Environment variables to inject into the agent's terminal session
//!
//! A cache mount is a redirect from the usual build output location to a location on the
//! persistent storage volume. On Linux, this is typically a bind mount. On macOS, it will be a
//! symbolic link.
//!
//! Currently, the implementation relies on [`spacectl`](https://github.com/namespacelabs/spacectl),
//! though this may change in the future.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use async_io::Timer;
use command::Stdio;
use command::r#async::Command;
use futures_lite::future;
use is_executable::IsExecutable as _;
use sha2::{Digest, Sha256};
use warp_errors::{ErrorExt, register_error};

pub mod spacectl;

use spacectl::{MountResponse, run_spacectl_mount};

const SPACECTL_TIMEOUT: Duration = Duration::from_secs(60);

/// Identifiers for a code repository.
///
/// These identifiers are used to scope repo-specific cache locations, such as
/// project-level build output directories.
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
            forge_host: forge_host.into().trim().to_ascii_lowercase(),
            owner: owner.into().trim().to_ascii_lowercase(),
            repo: repo.into().trim().to_ascii_lowercase(),
        }
    }
}

/// Key for scoping per-repository build caches. Cache keys are derived from [`RepoIdentity`] values.
///
/// The key format is opaque, but should be as stable as possible. Any changes make existing cache
/// data inaccessible.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("repository cache keys must be exactly 64 lowercase hexadecimal characters")]
pub struct InvalidRepoCacheKey;

/// Ownership scope for a given cache mount.
///
/// The scope determines whether multiple repositories using the same toolchain may share their
/// cache or not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheScope {
    /// This cache mount is specific to a particular repository (e.g. a Rust `./target` directory).
    Repository { name: String, key: RepoCacheKey },
    /// This cache mount is global to the system (e.g. the Homebrew download cache).
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

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RepositoryCacheSource {
    pub name: String,
    pub identity: RepoIdentity,
    pub cwd: PathBuf,
}

/// Description of the caches to configure in a particular scope.
///
/// For repository scopes, this is essentially the cache plan for that repository. For the global
/// scope, this is a synthetic combination of all caches to enable in the sandbox.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheConfiguration {
    /// Scope of this configuration.
    pub scope: CacheScope,
    /// The working directory to resolve cache source locations from. For repository scopes, this
    /// is the repository root. For global configurations, this has no meaningful value and is set
    /// to a temporary directory.
    pub cwd: PathBuf,
    /// Directory to store this configuration's cache data in within the persistent storage volume.
    pub relative_cache_dir: PathBuf,
    /// Set of cache modes to enable. Each cache mode corresponds to a tool, package manager, or
    /// language runtime. Values are generally stable, but should be considered opaque.
    pub modes: Vec<String>,
}

/// An executable plan for setting up build caches on the current host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheSetupPlan {
    /// The location of the persistent cache volume.
    pub cache_root: PathBuf,
    /// Individual cache configurations. This contains one or more repository-specific
    /// configurations followed by a global configuration.
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

    /// Validate that a cache plan is valid. A valid plan:
    /// * Contains one or more cache configurations
    /// * Ends in a globally-scoped cache configuration
    /// * Lists each repository exactly once, in order by cache key
    /// * Only uses safe cache locations within the cache volume (no absolute paths, `..`, or `.` components)
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
                || !configuration
                    .modes
                    .windows(2)
                    .all(|window| window[0] < window[1])
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

/// Report from preparing caches for a particular scope. This corresponds to a single call to
/// `spacectl cache mount`. The content is highly coupled to `spacectl`, and should only be used
/// for telemetry.
#[derive(Clone, Debug)]
pub struct CachePreparationReport {
    pub scope: CacheScope,
    pub modes: Vec<String>,
    pub relative_cache_dir: PathBuf,
    pub response: Option<MountResponse>,
    pub error: Option<CacheSetupError>,
    pub duration: Duration,
    pub mode_stats: BTreeMap<String, ModeCacheStats>,
}

impl CachePreparationReport {
    pub fn succeeded(&self) -> bool {
        self.error.is_none() && self.response.is_some()
    }
}

/// Report from preparing caches for an environment.
#[derive(Clone, Debug, Default)]
pub struct CacheSetupReport {
    pub plan: Option<CacheSetupPlan>,
    pub invocations: Vec<CachePreparationReport>,
    pub add_envs: BTreeMap<String, String>,
}

impl CacheSetupReport {
    /// List scoped cache setups which could not be mounted successfully.
    pub fn degradations(&self) -> impl Iterator<Item = &CachePreparationReport> {
        self.invocations
            .iter()
            .filter(|invocation| invocation.error.is_some())
    }
}

#[derive(Clone)]
struct DetectedCacheModes {
    source: RepositoryCacheSource,
    key: RepoCacheKey,
    modes: Vec<String>,
}

/// Calculates cache modes corresponding to global tools like package managers, which are not
/// detected from an individual repo.
pub fn global_cache_modes() -> Vec<String> {
    let mut modes = Vec::new();
    if cfg!(target_os = "linux") && has_command("apt-config") {
        modes.push("apt".to_owned());
    }
    if cfg!(target_os = "macos") && has_command("brew") {
        modes.push("brew".to_owned());
    }
    modes
}

/// Check if `command` exists on the current `$PATH`.
fn has_command(command: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(command).is_executable())
    })
}

/// Default implementation of the `run_command` hook for [`setup_cache`].
pub async fn default_run_command(command: Command) -> Result<Vec<u8>, CacheSetupError> {
    run_command_with_timeout(command, SPACECTL_TIMEOUT).await
}

/// Run a [`Command`] with a timeout. If the timeout expires or the future is dropped, then the
/// process is forcibly killed.
async fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<Vec<u8>, CacheSetupError> {
    command
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let output = async {
        command
            .output()
            .await
            .map_err(|_| CacheSetupError::SpawnFailed)
    };
    let timeout = async {
        Timer::after(timeout).await;
        Err(CacheSetupError::Timeout)
    };
    let output = future::race(output, timeout).await?;
    if !output.status.success() {
        return Err(CacheSetupError::NonzeroExit {
            exit_code: output.status.code(),
        });
    }
    Ok(output.stdout)
}

/// Set up build caching on the current host. See the crate-level documentation for a description
/// of the setup process. This configures caches for all detected repositories, as well as any
/// given global modes.
///
/// This should only be called once per sandbox, as it modifies shared filesystem locations.
/// The calling process need not run with superuser privileges, but the implementation may
/// escalate privileges with `sudo` or similar.
#[tracing::instrument(name = "setup_caches", skip_all, fields(tags.cloud_agent = true))]
pub async fn setup_cache<F, Fut>(
    cache_root: PathBuf,
    repositories: Vec<RepositoryCacheSource>,
    additional_global_modes: Vec<String>,
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
    keyed_repositories.sort();

    // Step 1: Detect the cache modes that apply to each repository. A mode corresponds to a tool
    // or language runtime, such as `apt-get` or Swift.
    let mut detected_modes = Vec::new();
    for (key, source) in keyed_repositories {
        let relative_cache_dir = PathBuf::from("repos").join(key.as_str());
        let configuration_root = cache_root.join(&relative_cache_dir);
        let scope = CacheScope::Repository {
            name: source.name.clone(),
            key: key.clone(),
        };

        // We create the scoped cache directory here, as `spacectl` fails if it doesn't exist.
        if std::fs::create_dir_all(&configuration_root).is_err() {
            report.invocations.push(failed_invocation(
                scope,
                Vec::new(),
                relative_cache_dir,
                CacheSetupError::RootCreationFailed,
                Duration::ZERO,
            ));
            continue;
        }

        // Run `spacectl` in dry-run mode, so that it detects all relevant cache modes.
        let invocation = run_spacectl_mount(
            scope,
            Vec::new(),
            true,
            relative_cache_dir,
            &configuration_root,
            &source.cwd,
            &mut run_command,
        )
        .await;
        if let Some(response) = &invocation.response {
            let modes = canonical_modes(response.input.modes.clone());
            if !modes.is_empty() {
                detected_modes.push(DetectedCacheModes { source, key, modes });
            }
        }
        report.invocations.push(invocation);
    }

    // Step 2: Given the per-repository results, construct the cache plan. This tells us which
    // caches to set up, and in what order.
    let plan = match construct_plan(cache_root, detected_modes, additional_global_modes) {
        Ok(Some(plan)) => plan,
        Ok(None) => return report,
        Err(error) => {
            report.invocations.push(failed_invocation(
                CacheScope::Global,
                Vec::new(),
                PathBuf::from("shared"),
                error,
                Duration::ZERO,
            ));
            return report;
        }
    };

    // Step 3: Run `spacectl cache mount` for real, setting up all the cache mounts.
    let mut repository_env = BTreeMap::new();
    let mut global_env = None;
    for configuration in &plan.configurations {
        let configuration_root = plan.cache_root.join(&configuration.relative_cache_dir);
        // All repo-scoped cache roots should already exist. However, we still need to create the global root.
        let invocation = if std::fs::create_dir_all(&configuration_root).is_err() {
            failed_invocation(
                configuration.scope.clone(),
                configuration.modes.clone(),
                configuration.relative_cache_dir.clone(),
                CacheSetupError::RootCreationFailed,
                Duration::ZERO,
            )
        } else {
            run_spacectl_mount(
                configuration.scope.clone(),
                configuration.modes.clone(),
                false,
                configuration.relative_cache_dir.clone(),
                &configuration_root,
                &configuration.cwd,
                &mut run_command,
            )
            .await
        };

        if let Some(response) = &invocation.response {
            match &configuration.scope {
                CacheScope::Repository { .. } => {
                    for (name, value) in &response.output.add_envs {
                        if repository_env.insert(name.clone(), value.clone()).is_some() {
                            log::warn!(
                                "repository build-cache environment conflict resolved by canonical repository order"
                            );
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

    // Step 4: Construct the merged environment variable map. If multiple repo-scoped cache
    // configurations set the same environment variable, we'll already have deduplicated them
    // (with last-repo-wins semantics) above. Here, we prefer using the globally-scoped set of
    // environment variables, but fall back to the combined set of repository environment variables.
    // We don't need to merge the two - the global cache configuration includes all modes set
    // by per-repo configurations, so it should have all the same variables.
    report.add_envs = global_env.unwrap_or(repository_env);
    report.add_envs.retain(|name, _| {
        if is_valid_env_name(name) {
            true
        } else {
            tracing::warn!(
                target: "build_cache",
                "ignored invalid build-cache environment variable name"
            );
            false
        }
    });
    report.plan = Some(plan);
    report
}

/// Construct a plan for setting up build caches on the current system. This requires:
/// - Analysis of the toolchains used in each repository (`detections`)
/// - System-level toolchains such as package managers
///
/// The cache plan consists of all per-repository cache configurations, sorted by cache key,
/// followed by a single global configuration. The global configuration includes all cache
/// modes across repos, plus any system-level modes.
///
/// Putting all repo-derived cache modes in the global configuration is counterintuitive, but
/// necessary for multi-repo environments. The problem is that a given cache mode might set
/// both globally-scoped mounts and repo-scoped mounts. For example, the `rust` mode caches
/// `~/.cargo/registry` and `./target`. Given an environment with two Rust projects, if we
/// simply mounted caches for each repo in sequence, then the `~/.cargo/registry` mount would
/// point to the scoped cache directory for an arbitrary repo.
///
/// Adding or removing repos from the
/// environment would therefore change the cache location and make prior data inaccessible.
/// Mounting all modes from each repo in an additional global scope means that the final cache
/// location for `~/.cargo/registry` is that globally-scoped location, which is independent of
/// the specific repos in the environment.
///
/// In the example above, the final cache structure would look something like:
/// - `~/.cargo/registry` => `/cache/shared/$HOME/.cargo/registry`
/// - `/workspace/repo-a/target` => `/cache/<repo-a-key>/target`
/// - `/workspace/repo-b/target` => `/cache/<repo-b-key>/target`
fn construct_plan(
    cache_root: PathBuf,
    mut detections: Vec<DetectedCacheModes>,
    additional_global_modes: Vec<String>,
) -> Result<Option<CacheSetupPlan>, CacheSetupError> {
    for detection in &mut detections {
        detection.modes = canonical_modes(std::mem::take(&mut detection.modes));
    }
    let mut global_modes = BTreeSet::new();
    for detection in &detections {
        global_modes.extend(detection.modes.iter().cloned());
    }
    global_modes.extend(additional_global_modes);
    if global_modes.is_empty() {
        return Ok(None);
    }

    // This is a quirk to accomodate the global cache scope described above. `spacectl` uses the
    // current working directory to resolve relative cache paths like `./target`. In the global
    // scope, we only care about the non-relative cache paths. To satisfy `spacectl`, we use a
    // throwaway temporary directory for setting up the global cache scope. This creates useless
    // mounts like `/tmp/abc123/target` => `/cache/shared/target`, but there's no harm in that.
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

/// Create a temporary scratch directory for setting up the global cache scope.
fn create_retained_scratch_directory<'a>(
    repository_paths: impl Iterator<Item = &'a Path>,
) -> Result<PathBuf, CacheSetupError> {
    let repository_paths = repository_paths.collect::<Vec<_>>();
    let mut builder = tempfile::Builder::new();
    builder.prefix("warp-spacectl-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    let directory = builder
        .tempdir()
        .map_err(|_| CacheSetupError::RootCreationFailed)?;
    if repository_paths
        .iter()
        .any(|repository| directory.path().starts_with(repository))
    {
        return Err(CacheSetupError::RootCreationFailed);
    }
    Ok(directory.keep())
}

fn failed_invocation(
    scope: CacheScope,
    modes: Vec<String>,
    relative_cache_dir: PathBuf,
    error: CacheSetupError,
    duration: Duration,
) -> CachePreparationReport {
    CachePreparationReport {
        scope,
        modes,
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

fn is_valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
