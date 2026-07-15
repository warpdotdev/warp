use std::path::Path;

use command::r#async::Command;
use serde::Deserialize;

use crate::CacheSetupError;

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

pub fn detect_command(cache_root: &Path, cwd: &Path) -> Command {
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

pub fn mount_command(cache_root: &Path, cwd: &Path, modes: &[String]) -> Command {
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

pub fn parse_mount_response(bytes: &[u8]) -> Result<MountResponse, CacheSetupError> {
    serde_json::from_slice(bytes).map_err(|_| CacheSetupError::JsonParseFailed)
}

#[cfg(test)]
#[path = "spacectl_tests.rs"]
mod tests;
