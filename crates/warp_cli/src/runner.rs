use clap::{ArgAction, ArgGroup, Args, Subcommand, ValueEnum};

use crate::json_filter::JsonOutput;
use crate::scope::ObjectScope;

/// Maximum length for runner descriptions.
const MAX_DESCRIPTION_LENGTH: usize = 240;

/// Validates that a description is within the allowed length.
fn validate_description(s: &str) -> Result<String, String> {
    let len = s.chars().count();
    if len > MAX_DESCRIPTION_LENGTH {
        Err(format!(
            "Description must be at most {MAX_DESCRIPTION_LENGTH} characters (got {len})"
        ))
    } else {
        Ok(s.to_string())
    }
}

/// Target operating system for a runner sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunnerOsArg {
    #[value(name = "linux")]
    Linux,
    #[value(name = "macos")]
    Macos,
}

/// Target CPU architecture for a runner sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunnerArchArg {
    /// Use the default architecture for the runner OS (x86-64 on Linux, aarch64 on macOS).
    #[value(name = "auto")]
    Auto,
    #[value(name = "x86-64")]
    X8664,
    #[value(name = "aarch64")]
    Aarch64,
}

/// macOS version to use for a runner sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunnerMacosVersionArg {
    #[value(name = "14")]
    Macos14,
    #[value(name = "15")]
    Macos15,
    #[value(name = "26")]
    Macos26,
    #[value(name = "27")]
    Macos27,
}

/// Sort-by values accepted by `--sort-by`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RunnerSortByArg {
    #[value(name = "name")]
    Name,
    #[value(name = "last-updated")]
    LastUpdated,
}

/// Runner-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum RunnerCommand {
    /// List runners.
    List(ListRunnersArgs),
    /// Create a new runner.
    Create(CreateRunnerArgs),
    /// Update an existing runner.
    Update(UpdateRunnerArgs),
    /// Delete a runner.
    Delete(DeleteRunnerArgs),
}

impl RunnerCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            RunnerCommand::List(_) => "runner list",
            RunnerCommand::Create(_) => "runner create",
            RunnerCommand::Update(_) => "runner update",
            RunnerCommand::Delete(_) => "runner delete",
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct ListRunnersArgs {
    /// Sort field.
    #[arg(long = "sort-by", value_enum, value_name = "FIELD")]
    pub sort_by: Option<RunnerSortByArg>,

    /// JSON formatting configuration.
    #[command(flatten)]
    pub json_output: JsonOutput,
}

#[derive(Debug, Clone, Args)]
pub struct CreateRunnerArgs {
    /// Name of the runner.
    #[arg(long = "name", short = 'n')]
    pub name: String,

    /// Description of the runner (max 240 characters).
    #[arg(long = "description", short = 'd', value_parser = validate_description)]
    pub description: Option<String>,

    /// Setup command to run when initializing the runner's sandbox (can be specified multiple times).
    #[arg(long = "setup-command", short = 'c', action = ArgAction::Append)]
    pub setup_command: Vec<String>,

    /// Target operating system for the runner.
    #[arg(long = "os", value_enum, default_value = "linux")]
    pub os: RunnerOsArg,

    /// Target CPU architecture for the runner. Defaults to the OS default (x86-64 on Linux, aarch64 on macOS).
    #[arg(long = "arch", value_enum, default_value = "auto")]
    pub arch: RunnerArchArg,

    /// Docker image reference for the sandbox (Linux only).
    #[arg(long = "docker-image")]
    pub docker_image: Option<String>,

    /// macOS version for the sandbox (macOS only).
    #[arg(long = "macos-version", value_enum)]
    pub macos_version: Option<RunnerMacosVersionArg>,

    /// Number of vCPUs for the instance shape. Requires --memory-gb.
    #[arg(long = "vcpus", requires = "memory_gb")]
    pub vcpus: Option<i32>,

    /// Memory in GB for the instance shape. Requires --vcpus.
    #[arg(long = "memory-gb", requires = "vcpus")]
    pub memory_gb: Option<i32>,

    #[command(flatten)]
    pub scope: ObjectScope,
}

#[derive(Debug, Clone, Args)]
// At least one of UID or --name is required. They are NOT mutually exclusive:
// when a UID is given, --name sets the runner's new name (rename); when no UID
// is given, --name is required and identifies the runner to update.
#[command(group(ArgGroup::new("runner_identifier").required(true).multiple(true).args(["id", "name"])))]
pub struct UpdateRunnerArgs {
    /// UID of the runner to update.
    #[arg(value_name = "UID")]
    pub id: Option<String>,

    /// New name for the runner when a UID is given (renames it). When no UID is
    /// given, this is required and used to locate the runner to update.
    #[arg(long = "name", short = 'n')]
    pub name: Option<String>,

    /// New description for the runner (max 240 characters).
    #[arg(long = "description", short = 'd', value_parser = validate_description)]
    pub description: Option<String>,

    /// Setup command to set on the runner (can be specified multiple times, replaces existing).
    #[arg(long = "setup-command", short = 'c', action = ArgAction::Append)]
    pub setup_command: Vec<String>,

    /// Target operating system for the runner.
    #[arg(long = "os", value_enum)]
    pub os: Option<RunnerOsArg>,

    /// Target CPU architecture for the runner.
    #[arg(long = "arch", value_enum)]
    pub arch: Option<RunnerArchArg>,

    /// Docker image reference for the sandbox (Linux only).
    #[arg(long = "docker-image")]
    pub docker_image: Option<String>,

    /// macOS version for the sandbox (macOS only).
    #[arg(long = "macos-version", value_enum)]
    pub macos_version: Option<RunnerMacosVersionArg>,

    /// Number of vCPUs for the instance shape. Can be set independently of --memory-gb (the other value is preserved).
    #[arg(long = "vcpus")]
    pub vcpus: Option<i32>,

    /// Memory in GB for the instance shape. Can be set independently of --vcpus (the other value is preserved).
    #[arg(long = "memory-gb")]
    pub memory_gb: Option<i32>,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteRunnerArgs {
    /// UID of the runner to delete.
    #[arg(value_name = "UID")]
    pub id: String,

    /// Delete without asking for confirmation.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

/// Validates that OS-specific options are only used with their matching `--os`.
///
/// Mirrors the server rule: Linux config (`--docker-image`) is only valid for
/// `--os linux`, and macOS config (`--macos-version`) is only valid for
/// `--os macos`.
pub fn validate_os_config(
    os: RunnerOsArg,
    docker_image: Option<&str>,
    macos_version: Option<RunnerMacosVersionArg>,
) -> Result<(), String> {
    match os {
        RunnerOsArg::Linux => {
            if macos_version.is_some() {
                return Err("--macos-version can only be used with --os macos".to_string());
            }
        }
        RunnerOsArg::Macos => {
            if docker_image.is_some() {
                return Err("--docker-image can only be used with --os linux".to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
