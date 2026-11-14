//! Decides whether a sub-project cache candidate needs a full `spacectl` dry run.
//!
//! Sub-projects only get their own cache configuration if they mount a destination inside the
//! repository that no earlier configuration claims (see `construct_plan`). Learning those
//! destinations requires a dry run, which plans every detected mode and runs tool queries such as
//! `cargo metadata` or `yarn config get`. When the detected modes can only produce destinations
//! outside the repository, or destinations an ancestor already claims, the configuration would be
//! pruned anyway and the cheaper `spacectl cache modes` detection is sufficient.
use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use command::r#async::Command;
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::CacheSetupError;
use crate::spacectl::SPACECTL_ENV;

/// Where a `spacectl` cache mode's mount destinations come from. Derived from the providers in
/// spacectl's `internal/cache/mode/providers.go` (v0.12).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModeLocality {
    /// Fixed paths, environment variables, or user-level tool configuration, none of which depend
    /// on the project.
    Global,
    /// User-level tool configuration that any of the given project files may override, e.g. a
    /// `cache` setting in `.npmrc`.
    ProjectConfigurable(&'static [&'static str]),
    /// Cargo's target directory, which workspace members share with their workspace root.
    CargoTarget,
    /// Paths that may be inside the project, e.g. `./vendor/bundle`.
    Repository,
}

/// Classifies `mode`. Modes this table does not know about may mount anything, so they are
/// treated as [`ModeLocality::Repository`].
pub(super) fn mode_locality(mode: &str) -> ModeLocality {
    classified_mode_locality(mode).unwrap_or(ModeLocality::Repository)
}

fn classified_mode_locality(mode: &str) -> Option<ModeLocality> {
    Some(match mode {
        "apt" | "brew" | "composer" | "deno" | "go" | "golangci-lint" | "gradle"
        | "kotlin-native" | "maven" | "mise" | "nix" | "playwright" | "python" | "tuist"
        | "xcode" => ModeLocality::Global,
        "bun" => ModeLocality::ProjectConfigurable(&["bunfig.toml"]),
        "npm" => ModeLocality::ProjectConfigurable(&[".npmrc"]),
        "pnpm" => ModeLocality::ProjectConfigurable(&[".npmrc", "pnpm-workspace.yaml"]),
        "poetry" => ModeLocality::ProjectConfigurable(&["poetry.toml"]),
        "uv" => ModeLocality::ProjectConfigurable(&["uv.toml", "pyproject.toml"]),
        // Yarn 2 and 3 default to a project-local cache, which they keep under `.yarn`.
        "yarn" => ModeLocality::ProjectConfigurable(&[".npmrc", ".yarn", ".yarnrc", ".yarnrc.yml"]),
        "rust" => ModeLocality::CargoTarget,
        "cocoapods" | "ruby" | "swiftpm" => ModeLocality::Repository,
        _ => return None,
    })
}

/// Members of the Cargo workspace containing a repository root, resolved at most once.
pub(super) struct CargoWorkspace {
    repository_root: PathBuf,
    member_manifests: OnceCell<Option<Arc<BTreeSet<PathBuf>>>>,
}

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
}

#[derive(Deserialize)]
struct CargoPackage {
    manifest_path: PathBuf,
}

impl CargoWorkspace {
    pub(super) fn new(repository_root: PathBuf) -> Self {
        Self {
            repository_root,
            member_manifests: OnceCell::new(),
        }
    }

    /// Whether `manifest` belongs to the workspace containing the repository root. Returns
    /// `false` if Cargo cannot describe that workspace.
    async fn has_member<F, Fut>(&self, manifest: &Path, run_command: &F) -> bool
    where
        F: Fn(Command) -> Fut,
        Fut: Future<Output = Result<Vec<u8>, CacheSetupError>>,
    {
        let members = self
            .member_manifests
            .get_or_init(|| self.load_member_manifests(run_command))
            .await;
        members
            .as_ref()
            .is_some_and(|members| members.contains(&canonical_or_original(manifest)))
    }

    async fn load_member_manifests<F, Fut>(&self, run_command: &F) -> Option<Arc<BTreeSet<PathBuf>>>
    where
        F: Fn(Command) -> Fut,
        Fut: Future<Output = Result<Vec<u8>, CacheSetupError>>,
    {
        let mut command = Command::new_with_process_group("cargo");
        command
            .args([
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--offline",
            ])
            .envs(SPACECTL_ENV.iter().copied())
            .current_dir(&self.repository_root);
        let metadata = match run_command(command).await {
            Ok(bytes) => serde_json::from_slice::<CargoMetadata>(&bytes).ok(),
            Err(error) => {
                tracing::info!(target: "build_cache", ?error, "cargo workspace metadata unavailable");
                None
            }
        }?;
        Some(Arc::new(
            metadata
                .packages
                .iter()
                .map(|package| canonical_or_original(&package.manifest_path))
                .collect(),
        ))
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}

/// Whether a sub-project at `cwd` needs a full dry run to learn its mount destinations, given the
/// modes `spacectl cache modes` detected there.
pub(super) async fn needs_dry_run<F, Fut>(
    modes: &[String],
    cwd: &Path,
    repository_root: &Path,
    cargo_workspace: &CargoWorkspace,
    run_command: &F,
) -> bool
where
    F: Fn(Command) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, CacheSetupError>>,
{
    for mode in modes {
        let needs_dry_run = match mode_locality(mode) {
            ModeLocality::Global => false,
            ModeLocality::ProjectConfigurable(files) => project_directories(cwd, repository_root)
                .any(|directory| files.iter().any(|file| directory.join(file).exists())),
            // A workspace member's target directory is the one the repository root claims, unless
            // Cargo configuration between the two relocates it.
            ModeLocality::CargoTarget => {
                project_directories(cwd, repository_root)
                    .filter(|directory| *directory != repository_root)
                    .any(|directory| {
                        ["config", "config.toml"]
                            .iter()
                            .any(|file| directory.join(".cargo").join(file).exists())
                    })
                    || !cargo_workspace
                        .has_member(&cwd.join("Cargo.toml"), run_command)
                        .await
            }
            ModeLocality::Repository => true,
        };
        if needs_dry_run {
            return true;
        }
    }
    false
}

/// `cwd` and its ancestors up to and including `repository_root`.
fn project_directories<'a>(
    cwd: &'a Path,
    repository_root: &'a Path,
) -> impl Iterator<Item = &'a Path> {
    cwd.ancestors()
        .take_while(move |directory| directory.starts_with(repository_root))
}

#[cfg(test)]
#[path = "sub_projects_tests.rs"]
mod tests;
