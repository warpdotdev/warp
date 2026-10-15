use std::collections::{HashMap, HashSet};
#[cfg(feature = "local_fs")]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(feature = "local_fs")]
use anyhow::bail;
use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};
use warp_cli::agent::RepositoryIdentity;

#[cfg(any(test, feature = "local_fs"))]
use super::environment_checkout_protocol::CheckoutBatch;
use super::environment_checkout_protocol::CheckoutRequest;

pub(super) const MODE_ENV: &str = "OZ_REPOSITORY_PREPARATION_MODE";
pub(super) const FILE_ENV: &str = "OZ_REPOSITORY_PREPARATION_FILE";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreparationRepository {
    pub source: RepositoryIdentity,
    pub request: CheckoutRequest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PreparationPolicy {
    pub task_id: String,
    pub execution_id: String,
    pub working_dir: PathBuf,
    pub supports_deferred_repositories: bool,
    pub eager: Vec<PreparationRepository>,
    pub deferred: Vec<PreparationRepository>,
}

impl PreparationPolicy {
    pub(super) fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            !self.task_id.is_empty() && !self.execution_id.is_empty(),
            "missing execution preparation binding"
        );
        ensure!(
            self.working_dir.is_absolute(),
            "preparation root must be absolute"
        );
        ensure!(
            self.supports_deferred_repositories || self.deferred.is_empty(),
            "disabled deferred preparation has repositories"
        );
        ensure!(
            self.eager.len() <= 128 && self.deferred.len() <= 128,
            "preparation repository limit exceeded"
        );
        let mut identities = HashSet::new();
        for record in self.eager.iter().chain(&self.deferred) {
            record.request.validate().map_err(anyhow::Error::msg)?;
            let original = CheckoutRequest {
                source: record.source.clone(),
                checkout_name: record.source.repo_name.clone(),
                ..record.request.clone()
            };
            original.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                record.request.checkout_name == record.source.repo_name,
                "preparation checkout must retain source name"
            );
            ensure!(
                identities.insert(record.source.identity()),
                "duplicate preparation identity"
            );
        }
        Ok(())
    }

    #[cfg(any(test, feature = "local_fs"))]
    pub(super) fn resolve(
        &self,
        batch: &mut CheckoutBatch,
        strict: bool,
        cleanup: bool,
    ) -> anyhow::Result<()> {
        self.validate()?;
        ensure!(
            batch.working_dir == self.working_dir,
            "checkout root does not match execution preparation"
        );
        ensure!(
            !strict || !cleanup,
            "strict checkout cannot clean up origins"
        );
        for request in &mut batch.repositories {
            if strict {
                let record = self
                    .deferred
                    .iter()
                    .find(|record| record.source.identity() == request.source.identity())
                    .context("repository is not in execution deferred preparation")?;
                let checkout_name = request.checkout_name.clone();
                *request = record.request.clone();
                request.checkout_name = checkout_name;
            } else {
                let permitted = self.eager.iter().any(|record| {
                    let mut eager = record.request.clone();
                    eager.remove_origin = false;
                    eager == *request && (!cleanup || record.request.remove_origin)
                });
                ensure!(
                    permitted,
                    "explicit checkout does not match execution eager preparation"
                );
            }
        }
        batch.validate().map_err(anyhow::Error::msg)?;
        Ok(())
    }
}

pub(super) struct PreparationFile {
    directory: tempfile::TempDir,
}

impl PreparationFile {
    pub(super) fn create(
        policy: &PreparationPolicy,
        task_id: &str,
        working_dir: &Path,
    ) -> anyhow::Result<Self> {
        policy.validate()?;
        ensure!(
            policy.task_id == task_id && policy.working_dir == working_dir,
            "execution preparation launch binding mismatch"
        );
        #[cfg(unix)]
        let directory = {
            use std::os::unix::fs::PermissionsExt as _;
            tempfile::Builder::new()
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir()?
        };
        #[cfg(not(unix))]
        let directory = tempfile::tempdir()?;
        let file = Self { directory };
        std::fs::write(file.path(), serde_json::to_vec(policy)?)?;
        Ok(file)
    }

    fn path(&self) -> PathBuf {
        self.directory.path().join("preparation.json")
    }
}

pub(super) fn launch_environment(
    policy: Option<&PreparationPolicy>,
    task_id: Option<&str>,
    working_dir: &Path,
    env: &mut HashMap<OsString, OsString>,
) -> anyhow::Result<Option<PreparationFile>> {
    for name in ["OZ_WORKSPACE_ROOT", "WARP_WORKSPACE_ROOT"] {
        env.insert(name.into(), working_dir.as_os_str().to_owned());
    }
    for name in ["OZ_RUN_ID", "WARP_RUN_ID"] {
        env.insert(name.into(), task_id.unwrap_or_default().into());
    }
    let file = match policy {
        Some(policy) => {
            let file = PreparationFile::create(
                policy,
                task_id.context("execution preparation requires a task")?,
                working_dir,
            )?;
            env.insert(MODE_ENV.into(), "execution".into());
            env.insert(FILE_ENV.into(), file.path().into_os_string());
            env.insert(
                "WARP_EXECUTION_ID".into(),
                policy.execution_id.clone().into(),
            );
            Some(file)
        }
        None => {
            env.insert(MODE_ENV.into(), "legacy".into());
            env.insert(FILE_ENV.into(), OsString::new());
            env.insert("WARP_EXECUTION_ID".into(), OsString::new());
            None
        }
    };
    Ok(file)
}

#[cfg(feature = "local_fs")]
pub(super) fn resolve_from_environment(
    batch: &mut CheckoutBatch,
    strict: bool,
    cleanup: bool,
) -> anyhow::Result<()> {
    let env: HashMap<_, _> = std::env::vars_os().collect();
    let value = |name: &str| env.get(OsStr::new(name)).filter(|value| !value.is_empty());
    let mode = value(MODE_ENV)
        .map(|value| value.to_str().context("invalid preparation mode encoding"))
        .transpose()?;
    match mode {
        Some("execution") => {
            let path = value(FILE_ENV).context("execution preparation file is missing")?;
            ensure!(
                Path::new(path).is_absolute(),
                "execution preparation path must be absolute"
            );
            let bytes = std::fs::read(path).context("could not read execution preparation")?;
            let policy: PreparationPolicy =
                serde_json::from_slice(&bytes).context("invalid execution preparation")?;
            for name in ["OZ_RUN_ID", "WARP_RUN_ID"] {
                ensure!(
                    value(name) == Some(&OsString::from(&policy.task_id)),
                    "execution preparation task mismatch"
                );
            }
            ensure!(
                value("WARP_EXECUTION_ID") == Some(&OsString::from(&policy.execution_id)),
                "execution preparation execution mismatch"
            );
            for name in ["OZ_WORKSPACE_ROOT", "WARP_WORKSPACE_ROOT"] {
                ensure!(
                    value(name).map(PathBuf::from).as_ref() == Some(&policy.working_dir),
                    "execution preparation root mismatch"
                );
            }
            ensure!(
                dunce::canonicalize(&policy.working_dir)? == policy.working_dir,
                "execution preparation root is not canonical"
            );
            policy.resolve(batch, strict, cleanup)
        }
        Some("legacy") => {
            ensure!(
                value(FILE_ENV).is_none() && value("WARP_EXECUTION_ID").is_none(),
                "legacy preparation inherited execution state"
            );
            Ok(())
        }
        None if value(FILE_ENV).is_none() && value("WARP_EXECUTION_ID").is_none() => Ok(()),
        Some(_) | None => bail!("execution preparation mode is missing or invalid"),
    }
}

#[cfg(test)]
#[path = "environment_preparation_tests.rs"]
mod tests;
