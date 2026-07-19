use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::{fmt, io};

use command::r#async::Command;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

const EXECUTABLE: &str = "spacectl";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SpacectlOperation {
    Mount,
    MountDetected,
}
impl SpacectlOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mount => "mount",
            Self::MountDetected => "mount-detected",
        }
    }
}

impl fmt::Display for SpacectlOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Error)]
pub(crate) enum SpacectlError {
    /// The spacectl process could not be started.
    #[error("spacectl cache {operation} command is unavailable")]
    CommandUnavailable {
        /// The attempted cache operation.
        operation: &'static str,
        /// The process-spawn error.
        #[source]
        source: io::Error,
    },
    /// The spacectl process exited unsuccessfully.

    #[error("spacectl cache {operation} command failed with status {status}")]
    CommandFailed {
        /// The attempted cache operation.
        operation: &'static str,
        /// The process exit status.
        status: ExitStatus,
    },
    /// Spacectl returned output that did not match its JSON contract.

    #[error("spacectl cache {operation} returned malformed JSON")]
    MalformedJson {
        /// The attempted cache operation.
        operation: &'static str,
        /// The JSON parsing error.
        #[source]
        source: serde_json::Error,
    },
    /// A cache mode name is empty or cannot be represented in a mode list.

    #[error("spacectl cache mode must not be empty, padded, or contain a comma")]
    InvalidMode,
    /// A mount command did not include any cache modes.

    #[error("at least one spacectl cache mode is required")]
    EmptyModes,
    /// A mount command used an empty or relative cache root.

    #[error("spacectl cache root must be a non-empty absolute path")]
    InvalidCacheRoot,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct SpacectlCacheMode(String);

impl SpacectlCacheMode {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, SpacectlError> {
        let value = value.into();
        if value.is_empty() || value.trim() != value || value.contains(',') {
            return Err(SpacectlError::InvalidMode);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct SpacectlCommand {
    operation: SpacectlOperation,
    arguments: Vec<OsString>,
    cwd: PathBuf,
}

impl SpacectlCommand {
    pub(super) fn mount(
        modes: &[SpacectlCacheMode],
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<Self, SpacectlError> {
        let modes = modes
            .iter()
            .map(SpacectlCacheMode::as_str)
            .collect::<BTreeSet<_>>();
        if modes.is_empty() {
            return Err(SpacectlError::EmptyModes);
        }
        validate_cache_root(cache_root)?;

        let mode_argument = format!("--mode={}", modes.into_iter().collect::<Vec<_>>().join(","));
        Ok(Self {
            operation: SpacectlOperation::Mount,
            arguments: vec![
                "cache".into(),
                "mount".into(),
                mode_argument.into(),
                "--dry_run=false".into(),
                "--cache_root".into(),
                cache_root.as_os_str().to_owned(),
                "-o".into(),
                "json".into(),
            ],
            cwd: cwd.to_path_buf(),
        })
    }

    pub(super) fn mount_detected(cache_root: &Path, cwd: &Path) -> Result<Self, SpacectlError> {
        validate_cache_root(cache_root)?;

        Ok(Self {
            operation: SpacectlOperation::MountDetected,
            arguments: vec![
                "cache".into(),
                "mount".into(),
                "--detect=*".into(),
                "--dry_run=false".into(),
                "--cache_root".into(),
                cache_root.as_os_str().to_owned(),
                "-o".into(),
                "json".into(),
            ],
            cwd: cwd.to_path_buf(),
        })
    }

    #[cfg(test)]
    pub(super) fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    #[cfg(test)]
    pub(super) fn cwd(&self) -> &Path {
        &self.cwd
    }
}

#[derive(Debug)]
pub(crate) struct Spacectl {
    executable: PathBuf,
}

impl Default for Spacectl {
    fn default() -> Self {
        Self {
            executable: EXECUTABLE.into(),
        }
    }
}

impl Spacectl {
    #[cfg(test)]
    pub(super) fn with_executable(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub(crate) async fn mount_cache(
        &self,
        modes: &[SpacectlCacheMode],
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, SpacectlError> {
        let command = SpacectlCommand::mount(modes, cache_root, cwd)?;
        let stdout = self.execute(&command).await?;
        parse_mount_response(&stdout)
    }

    pub(crate) async fn mount_detected_cache(
        &self,
        cache_root: &Path,
        cwd: &Path,
    ) -> Result<SpacectlMountResponse, SpacectlError> {
        let command = SpacectlCommand::mount_detected(cache_root, cwd)?;
        let stdout = self.execute(&command).await?;
        parse_mount_response_for_operation(&stdout, SpacectlOperation::MountDetected)
    }

    async fn execute(&self, command: &SpacectlCommand) -> Result<Vec<u8>, SpacectlError> {
        let output = Command::new(&self.executable)
            .args(&command.arguments)
            .current_dir(&command.cwd)
            .output()
            .await
            .map_err(|source| SpacectlError::CommandUnavailable {
                operation: command.operation.as_str(),
                source,
            })?;

        if !output.status.success() {
            log::warn!(
                "`spacectl cache {}` command failed with status {}",
                command.operation,
                output.status
            );
            return Err(SpacectlError::CommandFailed {
                operation: command.operation.as_str(),
                status: output.status,
            });
        }

        Ok(output.stdout)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SpacectlMountResponse {
    pub(crate) input_modes: Vec<SpacectlCacheMode>,
    pub(crate) add_envs: BTreeMap<String, String>,
    pub(crate) disk_usage: Option<SpacectlDiskUsage>,
    pub(crate) mounts: Vec<SpacectlMount>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct SpacectlDiskUsage {
    pub(crate) total: String,
    pub(crate) used: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct SpacectlMount {
    pub(crate) mode: SpacectlCacheMode,
    pub(crate) cache_path: String,
    pub(crate) mount_path: String,
    pub(crate) cache_hit: bool,
}

#[derive(Deserialize)]
struct RawMountResponse {
    input: RawMountInput,
    output: RawMountOutput,
}

#[derive(Deserialize)]
struct RawMountInput {
    #[serde(default)]
    modes: Vec<String>,
}

#[derive(Deserialize)]
struct RawMountOutput {
    #[serde(default)]
    add_envs: BTreeMap<String, String>,
    disk_usage: Option<SpacectlDiskUsage>,
    #[serde(default)]
    mounts: Vec<RawMount>,
}

#[derive(Deserialize)]
struct RawMount {
    mode: String,
    cache_path: String,
    mount_path: String,
    cache_hit: bool,
}

pub(super) fn parse_mount_response(output: &[u8]) -> Result<SpacectlMountResponse, SpacectlError> {
    parse_mount_response_for_operation(output, SpacectlOperation::Mount)
}

fn parse_mount_response_for_operation(
    output: &[u8],
    operation: SpacectlOperation,
) -> Result<SpacectlMountResponse, SpacectlError> {
    let output: RawMountResponse = parse_json(output, operation)?;
    let input_modes = output
        .input
        .modes
        .into_iter()
        .map(SpacectlCacheMode::new)
        .collect::<Result<_, _>>()?;
    let mounts = output
        .output
        .mounts
        .into_iter()
        .map(|mount| {
            Ok(SpacectlMount {
                mode: SpacectlCacheMode::new(mount.mode)?,
                cache_path: mount.cache_path,
                mount_path: mount.mount_path,
                cache_hit: mount.cache_hit,
            })
        })
        .collect::<Result<_, SpacectlError>>()?;

    Ok(SpacectlMountResponse {
        input_modes,
        add_envs: output.output.add_envs,
        disk_usage: output.output.disk_usage,
        mounts,
    })
}

fn parse_json<T: DeserializeOwned>(
    output: &[u8],
    operation: SpacectlOperation,
) -> Result<T, SpacectlError> {
    serde_json::from_slice(output).map_err(|source| SpacectlError::MalformedJson {
        operation: operation.as_str(),
        source,
    })
}

fn validate_cache_root(cache_root: &Path) -> Result<(), SpacectlError> {
    if cache_root.as_os_str().is_empty() || !cache_root.is_absolute() {
        return Err(SpacectlError::InvalidCacheRoot);
    }
    Ok(())
}

#[cfg(test)]
#[path = "spacectl_tests.rs"]
mod tests;
