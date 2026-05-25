//! Auth subcommands for `warpctrl`.
use std::io::Read as _;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use local_control::discovery::InstanceRecord;
use local_control::protocol::{ControlError, ErrorCode};
use local_control::scripting::{
    ApiKeySecret, ApiKeyStatus, ApiKeyStorageRef, AuthStatusSummary, exchange_api_key_stub,
};
use serde::Serialize;

use crate::agent::OutputFormat;
use crate::local_control::TargetArgs;
use crate::local_control::output::{write_json, write_json_line};

const API_KEY_REF_PATH_ENV: &str = "WARPCTRL_API_KEY_REF_PATH";

/// Authentication and scripting identity commands.
#[derive(Debug, Clone, Subcommand)]
pub enum AuthCommand {
    /// Report authenticated scripting status for the selected Warp app.
    Status(TargetArgs),

    /// Focus the selected Warp app's sign-in UI for interactive login.
    Login(TargetArgs),

    /// Manage external scripting API keys.
    #[command(subcommand)]
    ApiKey(ApiKeySubcommand),
}

/// API-key management subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ApiKeySubcommand {
    /// Store or reference an external Warp scripting API key.
    Set(ApiKeySetArgs),

    /// Show subject and scope metadata for the stored scripting API key.
    Status(TargetArgs),

    /// Delete the locally stored API-key reference.
    Revoke(TargetArgs),
}

/// Arguments for `warpctrl auth api-key set`.
#[derive(Debug, Clone, Args)]
#[group(required = true, multiple = false)]
pub struct ApiKeySourceArgs {
    /// Read the API key from this environment variable.
    #[arg(long = "key-env", value_name = "ENV_VAR")]
    pub key_env: Option<String>,

    /// Read the API key from stdin.
    #[arg(long = "key-stdin")]
    pub key_stdin: bool,
}

/// Arguments for `warpctrl auth api-key set`.
#[derive(Debug, Clone, Args)]
pub struct ApiKeySetArgs {
    #[command(flatten)]
    pub target: TargetArgs,

    #[command(flatten)]
    pub source: ApiKeySourceArgs,
}

#[derive(Serialize)]
struct ApiKeyRevokeSummary {
    revoked_local_reference: bool,
    server_side_revocation_supported: bool,
}

pub(super) fn run_auth_command(
    command: AuthCommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        AuthCommand::Status(args) => {
            let instance = super::commands::select_target_instance(args)?;
            write_auth_status(&instance, output_format)
        }
        AuthCommand::Login(args) => {
            let _instance = super::commands::select_target_instance(args)?;
            Err(ControlError::new(
                ErrorCode::UnsupportedAction,
                "auth login requires the app-side sign-in surface action, which is not implemented on this base",
            ))
        }
        AuthCommand::ApiKey(command) => run_api_key_command(command, output_format),
    }
}

fn run_api_key_command(
    command: ApiKeySubcommand,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    match command {
        ApiKeySubcommand::Set(args) => {
            let _instance = super::commands::select_target_instance(args.target)?;
            let secret = read_api_key_secret(args.source)?;
            let storage_ref = exchange_api_key_stub(&secret)?;
            save_api_key_storage_ref(&storage_ref)?;
            write_api_key_status(storage_ref.status(), output_format)
        }
        ApiKeySubcommand::Status(args) => {
            let _instance = super::commands::select_target_instance(args)?;
            write_api_key_status(load_api_key_status(), output_format)
        }
        ApiKeySubcommand::Revoke(args) => {
            let _instance = super::commands::select_target_instance(args)?;
            let revoked_local_reference = remove_api_key_storage_ref()?;
            let summary = ApiKeyRevokeSummary {
                revoked_local_reference,
                server_side_revocation_supported: false,
            };
            write_output(&summary, output_format)
        }
    }
}

fn write_auth_status(
    instance: &InstanceRecord,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    let summary = AuthStatusSummary {
        instance_id: instance.instance_id.0.clone(),
        local_control_enabled: instance.outside_warp_control_enabled,
        app_user_logged_in: false,
        app_user_subject: None,
        outside_warp_authenticated_grants_enabled: false,
        api_key_status: load_api_key_status(),
    };
    write_output(&summary, output_format)
}

fn write_api_key_status(
    status: ApiKeyStatus,
    output_format: OutputFormat,
) -> Result<(), ControlError> {
    write_output(&status, output_format)
}

fn write_output(value: &impl Serialize, output_format: OutputFormat) -> Result<(), ControlError> {
    match output_format {
        OutputFormat::Json => write_json(value),
        OutputFormat::Ndjson => write_json_line(value),
        OutputFormat::Pretty | OutputFormat::Text => write_json(value),
    }
}

fn read_api_key_secret(source: ApiKeySourceArgs) -> Result<ApiKeySecret, ControlError> {
    if let Some(env_var) = source.key_env {
        let value = std::env::var(&env_var).map_err(|_| {
            ControlError::new(
                ErrorCode::InvalidParams,
                format!("environment variable {env_var} is not set"),
            )
        })?;
        return ApiKeySecret::new(value);
    }
    let mut value = String::new();
    std::io::stdin().read_to_string(&mut value).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidParams,
            "failed to read API key from stdin",
            err.to_string(),
        )
    })?;
    ApiKeySecret::new(value.trim_end_matches(['\r', '\n']).to_owned())
}

fn load_api_key_status() -> ApiKeyStatus {
    match load_api_key_storage_ref() {
        Ok(Some(storage_ref)) => storage_ref.status(),
        Ok(None) | Err(_) => ApiKeyStatus::NotConfigured,
    }
}

fn load_api_key_storage_ref() -> Result<Option<ApiKeyStorageRef>, ControlError> {
    let path = api_key_ref_path();
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to read warpctrl API-key reference",
            err.to_string(),
        )
    })?;
    let storage_ref = serde_json::from_str(&contents).map_err(|err| {
        ControlError::with_details(
            ErrorCode::InvalidRequest,
            "failed to decode warpctrl API-key reference",
            err.to_string(),
        )
    })?;
    Ok(Some(storage_ref))
}

fn save_api_key_storage_ref(storage_ref: &ApiKeyStorageRef) -> Result<(), ControlError> {
    let path = api_key_ref_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to create warpctrl API-key reference directory",
                err.to_string(),
            )
        })?;
        set_private_dir_permissions(parent);
    }
    let bytes = serde_json::to_vec_pretty(storage_ref).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to encode warpctrl API-key reference",
            err.to_string(),
        )
    })?;
    std::fs::write(&path, bytes).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to write warpctrl API-key reference",
            err.to_string(),
        )
    })?;
    set_private_file_permissions(&path);
    Ok(())
}

fn remove_api_key_storage_ref() -> Result<bool, ControlError> {
    let path = api_key_ref_path();
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(path).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to remove warpctrl API-key reference",
            err.to_string(),
        )
    })?;
    Ok(true)
}

fn api_key_ref_path() -> PathBuf {
    if let Some(path) = std::env::var_os(API_KEY_REF_PATH_ENV) {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home)
        .join(".warp")
        .join("warpctrl")
        .join("api-key-ref.json")
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &std::path::Path) {}

#[cfg(unix)]
fn set_private_file_permissions(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;

    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &std::path::Path) {}
