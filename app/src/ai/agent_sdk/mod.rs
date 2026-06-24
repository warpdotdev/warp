//! Agent SDK entry points for invoking Agent-related functionality from the app.
//! For now this provides a simple runner that echoes the received command.

use std::fmt::Write;

use telemetry::CliTelemetryEvent;
use warp_cli::api_key::ApiKeyCommand;
use warp_cli::artifact::ArtifactCommand;
use warp_cli::environment::{EnvironmentCommand, ImageCommand};
use warp_cli::integration::IntegrationCommand;
use warp_cli::mcp::MCPCommand;
use warp_cli::model::ModelCommand;
use warp_cli::provider::ProviderCommand;
use warp_cli::secret::SecretCommand;
use warp_cli::{CliCommand, GlobalOptions};
use warp_core::features::FeatureFlag;
use warp_graphql::object_permissions::OwnerType;
#[cfg(not(target_family = "wasm"))]
use warp_logging::log_file_path;
use warpui::platform::TerminationMode;
use warpui::AppContext;
use warpui::SingletonEntity;

use crate::ai::skills::ResolveSkillError;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::AuthStateProvider;
use crate::send_telemetry_sync_from_app_ctx;

mod api_key;
mod artifact;
pub(crate) mod artifact_upload;
mod common;
mod config_file;
mod environment;
#[cfg(not(target_family = "wasm"))]
mod integration;
#[cfg(not(target_family = "wasm"))]
mod integration_output;
mod mcp;
mod mcp_config;
mod model;
mod oauth_flow;
pub mod output;
mod provider;
pub(crate) mod retry;
mod secret;
pub(crate) mod setup_observability;
mod telemetry;
#[cfg(test)]
mod test_support;
mod text_layout;

/// Prints a non-blocking warning to stderr when the CLI is invoked with a team-scoped API key.
fn maybe_warn_team_api_key(ctx: &AppContext) {
    let auth_state = AuthStateProvider::handle(ctx).as_ref(ctx).get();
    let owner_type = auth_state.api_key_owner_type();
    if !matches!(owner_type, Some(OwnerType::Team)) {
        return;
    }

    eprintln!(
        "\x1b[33mWarning: Free cloud credits apply to personal runs only but this run uses \
         a team API key. If you want to use free cloud credits, consider using a personal API key instead.\x1b[0m"
    );
}

/// Run a Warp CLI command.
#[tracing::instrument(name = "agent_sdk::run", skip_all, err, fields(tags.cloud_agent = true))]
pub fn run(
    ctx: &mut AppContext,
    command: CliCommand,
    global_options: GlobalOptions,
) -> anyhow::Result<()> {
    let event = command_to_telemetry_event(&command);
    send_telemetry_sync_from_app_ctx!(event, ctx);

    launch_command(ctx, command, global_options)
}

/// Dispatch a CLI command to its handler.
fn dispatch_command(
    ctx: &mut AppContext,
    command: CliCommand,
    global_options: GlobalOptions,
) -> anyhow::Result<()> {
    match command {
        CliCommand::Environment(environment_cmd) => {
            if !FeatureFlag::CloudEnvironments.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'environment'"));
            }
            environment::run(ctx, global_options, environment_cmd)
        }
        CliCommand::MCP(mcp_cmd) => mcp::run(ctx, global_options, mcp_cmd),
        CliCommand::Model(model_cmd) => model::run(ctx, global_options, model_cmd),
        CliCommand::Provider(provider_cmd) => {
            if !FeatureFlag::ProviderCommand.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'provider'"));
            }
            provider::run(ctx, global_options, provider_cmd)
        }
        #[cfg(not(target_family = "wasm"))]
        CliCommand::Integration(integration_cmd) => {
            if !FeatureFlag::IntegrationCommand.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'integration'"));
            }
            integration::run(ctx, global_options, integration_cmd)
        }
        #[cfg(target_family = "wasm")]
        CliCommand::Integration(_) => {
            return Err(anyhow::anyhow!("invalid value 'integration'"));
        }
        CliCommand::Secret(secret_cmd) => {
            if !FeatureFlag::WarpManagedSecrets.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'secret'"));
            }
            secret::run(ctx, global_options, secret_cmd)
        }
        CliCommand::Artifact(artifact_cmd) => {
            if !FeatureFlag::ArtifactCommand.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'artifact'"));
            }
            artifact::run(ctx, global_options, artifact_cmd)
        }
        CliCommand::ApiKey(api_key_cmd) => {
            if !FeatureFlag::APIKeyManagement.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'api-key'"));
            }
            api_key::run(ctx, global_options, api_key_cmd)
        }
    }
}

fn format_skill_resolution_error(err: ResolveSkillError) -> String {
    match err {
        ResolveSkillError::NotFound { skill } => {
            format!("Skill '{skill}' not found")
        }
        ResolveSkillError::RepoNotFound { repo } => {
            format!("Repository '{repo}' not found")
        }
        ResolveSkillError::Ambiguous { skill, candidates } => {
            let mut msg = format!(
                "Skill '{skill}' is ambiguous; specify as repo:skill_name\n\nCandidates:\n"
            );
            for path in candidates {
                msg.push_str(&format!("- {}\n", path.display()));
            }
            msg
        }
        ResolveSkillError::OrgMismatch {
            repo,
            expected,
            found,
        } => {
            format!("Repository '{repo}' found but belongs to org '{found}', expected '{expected}'")
        }
        ResolveSkillError::ParseFailed { path, message } => {
            format!("Failed to parse skill file {}: {message}", path.display())
        }
        ResolveSkillError::CloneFailed { org, repo, message } => {
            format!("Failed to clone repository '{org}/{repo}': {message}")
        }
    }
}

/// Returns `true` if the given CLI command requires authentication.
fn command_requires_auth(command: &CliCommand) -> bool {
    match command {
        CliCommand::Environment(environment_cmd) => match environment_cmd {
            EnvironmentCommand::List => true,
            EnvironmentCommand::Create { .. } => true,
            EnvironmentCommand::Delete { .. } => true,
            EnvironmentCommand::Update { .. } => true,
            EnvironmentCommand::Get { .. } => true,
            EnvironmentCommand::Image(ImageCommand::List) => true,
        },
        CliCommand::MCP(mcp_cmd) => match mcp_cmd {
            MCPCommand::List => true,
        },
        CliCommand::Model(model_cmd) => match model_cmd {
            ModelCommand::List => true,
        },
        CliCommand::Provider(_) => true,
        CliCommand::Integration(_) => true,
        CliCommand::Secret(_) => true,
        CliCommand::Artifact(_) => true,
        CliCommand::ApiKey(_) => true,
    }
}

/// Launch a CLI command, checking authentication first if needed.
///
/// If auth is not required, dispatches the command immediately.
/// If auth is required and the user is logged in, triggers a user refresh
/// before launching the command.
fn launch_command(
    ctx: &mut AppContext,
    command: CliCommand,
    global_options: GlobalOptions,
) -> anyhow::Result<()> {
    let requires_auth = command_requires_auth(&command);

    if !requires_auth {
        return dispatch_command(ctx, command, global_options);
    }

    let cli_name = warp_cli::binary_name().unwrap_or_else(|| "warp".to_string());

    let auth_state = AuthStateProvider::handle(ctx).as_ref(ctx).get();
    if !auth_state.is_logged_in() {
        return Err(anyhow::anyhow!(
            "You are not logged in - please log in with `{cli_name} login` to continue."
        ));
    }

    // User is logged in — subscribe to auth events, trigger a refresh, and wait
    // for the result before running the command.
    let mut dispatched = false;
    ctx.subscribe_to_model(&AuthManager::handle(ctx), move |_, event, ctx| {
        if dispatched {
            return;
        }
        match event {
            AuthManagerEvent::AuthComplete => {
                dispatched = true;
                if let Err(err) = dispatch_command(ctx, command.clone(), global_options.clone()) {
                    report_fatal_error(err, ctx);
                }
            }
            AuthManagerEvent::NeedsReauth => {
                dispatched = true;
                let auth_state = AuthStateProvider::handle(ctx).as_ref(ctx).get();
                let message = if auth_state.is_api_key_authenticated() {
                    "Your API key is invalid. Please provide a valid key via '--api-key' or the ZERP_API_KEY environment variable.".to_string()
                } else {
                    format!("Your credentials are invalid. Please log in again with `{cli_name} login`.")
                };
                report_fatal_error(anyhow::anyhow!(message), ctx);
            }
            AuthManagerEvent::AuthFailed(err) => {
                dispatched = true;
                report_fatal_error(anyhow::anyhow!("Authentication failed: {err:#}"), ctx);
            }
            _ => {}
        }
    });

    // Trigger the user refresh - the subscription above will handle the result.
    AuthManager::handle(ctx).update(ctx, |auth_manager: &mut AuthManager, ctx| {
        auth_manager.refresh_user(ctx);
    });

    Ok(())
}

/// Check if we're running within Warp (for example, if this is an invocation of the Warp CLI
/// within a Warp terminal session).
pub fn is_running_in_warp() -> bool {
    std::env::var("TERM_PROGRAM")
        .map(|v| v == "WarpTerminal")
        .unwrap_or(false)
}

/// Report a fatal error and terminate the app.
fn report_fatal_error(err: anyhow::Error, ctx: &mut AppContext) {
    let mut message = err.to_string();
    for cause in err.chain().skip(1) {
        let _ = write!(&mut message, "\n=> {cause}");
    }

    tracing::event!(tracing::Level::ERROR, tags.cloud_agent = true, message);

    #[cfg(not(target_family = "wasm"))]
    {
        if let Ok(path) = log_file_path() {
            let _ = write!(
                message,
                "\n\nFor more information, check Warp logs at {}",
                path.display()
            );
        }
    }

    let error = anyhow::anyhow!(message);
    ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
}

/// Map each CLI command into a telemetry event to emit when it's executed.
fn command_to_telemetry_event(command: &CliCommand) -> CliTelemetryEvent {
    match command {
        CliCommand::Environment(EnvironmentCommand::List) => CliTelemetryEvent::EnvironmentList,
        CliCommand::Environment(EnvironmentCommand::Create { .. }) => {
            CliTelemetryEvent::EnvironmentCreate
        }
        CliCommand::Environment(EnvironmentCommand::Delete { .. }) => {
            CliTelemetryEvent::EnvironmentDelete
        }
        CliCommand::Environment(EnvironmentCommand::Update { .. }) => {
            CliTelemetryEvent::EnvironmentUpdate
        }
        CliCommand::Environment(EnvironmentCommand::Get { .. }) => {
            CliTelemetryEvent::EnvironmentGet
        }
        CliCommand::Environment(EnvironmentCommand::Image(ImageCommand::List)) => {
            CliTelemetryEvent::EnvironmentImageList
        }
        CliCommand::MCP(MCPCommand::List) => CliTelemetryEvent::MCPList,
        CliCommand::Model(ModelCommand::List) => CliTelemetryEvent::ModelList,
        CliCommand::Provider(ProviderCommand::Setup(_)) => CliTelemetryEvent::ProviderSetup,
        CliCommand::Provider(ProviderCommand::List) => CliTelemetryEvent::ProviderList,
        CliCommand::Integration(integration_cmd) => match integration_cmd {
            IntegrationCommand::Create(_) => CliTelemetryEvent::IntegrationCreate,
            IntegrationCommand::Update(_) => CliTelemetryEvent::IntegrationUpdate,
            IntegrationCommand::List => CliTelemetryEvent::IntegrationList,
        },
        CliCommand::Secret(secret_cmd) => match secret_cmd {
            SecretCommand::Create(_) => CliTelemetryEvent::SecretCreate,
            SecretCommand::Delete(_) => CliTelemetryEvent::SecretDelete,
            SecretCommand::Update(_) => CliTelemetryEvent::SecretUpdate,
            SecretCommand::List(_) => CliTelemetryEvent::SecretList,
        },
        CliCommand::Artifact(artifact_cmd) => match artifact_cmd {
            ArtifactCommand::Upload(_) => CliTelemetryEvent::ArtifactUpload,
            ArtifactCommand::Get(_) => CliTelemetryEvent::ArtifactGet,
            ArtifactCommand::Download(_) => CliTelemetryEvent::ArtifactDownload,
        },
        CliCommand::ApiKey(api_key_cmd) => match api_key_cmd {
            ApiKeyCommand::List(_) => CliTelemetryEvent::ApiKeyList,
            ApiKeyCommand::Create(_) => CliTelemetryEvent::ApiKeyCreate,
            ApiKeyCommand::Expire(_) => CliTelemetryEvent::ApiKeyExpire,
        },
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
