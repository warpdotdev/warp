//! Agent SDK entry points for invoking Agent-related functionality from the app.
//! For now this provides a simple runner that echoes the received command.

use std::collections::HashMap;
use std::fmt::Write;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ai::api_keys::{ApiKeyManager, AwsCredentialsRefreshStrategy};
use anyhow::Context;
pub use driver::AgentDriver;
use driver::AgentDriverError;
pub(crate) use driver::harness::{ClaudeHarness, task_env_vars, validate_cli_installed};
use telemetry::CliTelemetryEvent;
use tracing::Instrument as _;
use warp_cli::agent::{
    AgentCommand, AgentProfileCommand, Harness, HarnessTransport, OutputFormat, Prompt,
    RunAgentArgs,
};
use warp_cli::api_key::ApiKeyCommand;
use warp_cli::artifact::ArtifactCommand;
use warp_cli::environment::{EnvironmentCommand, ImageCommand};
use warp_cli::federate::FederateCommand;
use warp_cli::harness_support::{HarnessSupportCommand, ReportArtifactCommand, TaskStatus};
use warp_cli::integration::IntegrationCommand;
use warp_cli::mcp::MCPCommand;
use warp_cli::memory_store::{MemoryCommand, MemoryStoreCommand};
use warp_cli::model::ModelCommand;
use warp_cli::provider::ProviderCommand;
use warp_cli::runner::RunnerCommand;
use warp_cli::schedule::ScheduleSubcommand;
use warp_cli::secret::SecretCommand;
use warp_cli::share::ShareRequest;
use warp_cli::skill::SkillSpec;
use warp_cli::task::{MessageCommand, TaskCommand};
use warp_cli::{CliCommand, GlobalOptions, OZ_HARNESS_ENV};
use warp_core::features::FeatureFlag;
use warp_errors::report_error;
use warp_graphql::object_permissions::OwnerType;
use warp_graphql::queries::execution_config::ExecutionConfiguration;
use warp_isolation_platform::IsolationPlatformError;
#[cfg(not(target_family = "wasm"))]
use warp_logging::log_file_path;
use warp_managed_secrets::{ManagedSecretValue, convert_task_secrets};
use warp_multi_agent_api::ConversationData;
use warp_server_client::iap::{IapManager, IapManagerEvent};
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelSpawner, SingletonEntity};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::api::convert_conversation::{
    RestorationMode, convert_conversation_data_to_ai_conversation,
};
use crate::ai::agent::conversation::{AIConversationId, ServerAIConversationMetadata};
use crate::ai::agent_sdk::driver::harness::{HarnessKind, harness_kind};
use crate::ai::agent_sdk::driver::{AgentDriverOptions, AgentRunPrompt, Task};
use crate::ai::agent_sdk::mcp_config::build_mcp_servers_from_specs;
use crate::ai::agent_sdk::setup_observability::{
    OzRunTimelineEvent, SetupClientEventReporter, SetupStep,
};
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::ambient_agents::task::{HarnessConfig, TaskAttachment};
use crate::ai::attachment_utils::attachments_download_dir;
#[cfg(not(target_family = "wasm"))]
use crate::ai::aws_credentials::{BedrockOidcCredentialsConfig, refresh_aws_credentials_oidc};
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironment, SourceRepo,
};
use crate::ai::llms::LLMId;
use crate::ai::skills::{
    ResolveSkillError, ResolvedSkill, clone_repo_for_skill, resolve_skill_spec,
};
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::cloud_object::CloudObjectLookup as _;
use crate::cloud_object::model::persistence::CloudModel;
use crate::send_telemetry_sync_from_app_ctx;
use crate::server::ids::{ServerId, SyncId};
use crate::server::retry_strategies::with_retry;
use crate::server::server_api::ai::{
    AIClient, AgentConfigSnapshot, ConversationNotFound, GitCredential, TaskGitCredentialsError,
};
use crate::server::server_api::managed_secrets::AppManagedSecretManager as ManagedSecretManager;
use crate::server::server_api::{ServerApi, ServerApiProvider};
use crate::server::team_scope::RequestTeamScope;
use crate::terminal::view::ConversationRestorationInNewPaneType;
use crate::workflows::workflow::Workflow;
use crate::workspaces::user_workspaces::HeadlessTeamScope;

mod admin;
mod agent_config;
mod agent_management;
mod ambient;
mod api_key;
mod artifact;
pub(crate) mod artifact_upload;
mod common;
mod config_file;
pub(crate) mod driver;
mod environment;
pub(crate) mod environment_snapshot;
mod execution_config;
mod federate;
mod harness_support;
#[cfg(not(target_family = "wasm"))]
mod integration;
#[cfg(not(target_family = "wasm"))]
mod integration_output;
mod mcp;
mod mcp_config;
mod memory_store;
mod model;
mod oauth_flow;
pub mod output;
mod profiles;
mod provider;
pub(crate) mod retry;
mod runner;
mod schedule;
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

fn bedrock_oidc_credentials_config(
    options: &AgentDriverOptions,
    role_arn: String,
    region: String,
) -> Result<BedrockOidcCredentialsConfig, AgentDriverError> {
    let task_id = options.task_id.ok_or_else(|| {
        AgentDriverError::AwsBedrockCredentialsFailed(
            "AWS Bedrock inference requires an ambient task ID before credentials can be minted"
                .to_string(),
        )
    })?;
    Ok(BedrockOidcCredentialsConfig {
        task_id: task_id.to_string(),
        role_arn,
        region,
    })
}

pub(crate) fn run_environment_checkout(
    args: &warp_cli::environment_checkout::EnvironmentCheckoutArgs,
) -> anyhow::Result<()> {
    #[cfg(feature = "local_fs")]
    {
        driver::environment_checkout::run(args)
    }
    #[cfg(not(feature = "local_fs"))]
    {
        let _ = args;
        Err(anyhow::anyhow!(
            "environment checkout requires filesystem support"
        ))
    }
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
        CliCommand::Agent(agent_cmd) => run_agent(ctx, global_options, agent_cmd),
        CliCommand::Environment(environment_cmd) => {
            if !FeatureFlag::CloudEnvironments.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'environment'"));
            }
            environment::run(ctx, global_options, environment_cmd)
        }
        CliCommand::EnvironmentCheckout(args) => run_environment_checkout(&args),
        CliCommand::MCP(mcp_cmd) => mcp::run(ctx, global_options, mcp_cmd),
        CliCommand::Run(task_cmd) => run_task(ctx, global_options, task_cmd),
        CliCommand::Model(model_cmd) => model::run(ctx, global_options, model_cmd),
        CliCommand::MemoryStore(memory_store_cmd) => {
            memory_store::run(ctx, global_options, memory_store_cmd)
        }
        CliCommand::Memory(memory_cmd) => memory_store::run_memory(ctx, global_options, memory_cmd),
        CliCommand::Login => admin::login(ctx),
        CliCommand::Logout => admin::logout(ctx),
        CliCommand::Whoami => admin::whoami(ctx, global_options.output_format),
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
        CliCommand::Schedule(schedule_cmd) => {
            if !FeatureFlag::ScheduledAmbientAgents.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'schedule'"));
            }
            schedule::run(ctx, global_options, schedule_cmd)
        }
        CliCommand::Secret(secret_cmd) => {
            if !FeatureFlag::WarpManagedSecrets.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'secret'"));
            }
            secret::run(ctx, global_options, secret_cmd)
        }
        CliCommand::Federate(federate_cmd) => {
            if !FeatureFlag::OzIdentityFederation.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'federate'"));
            }
            federate::run(ctx, global_options, federate_cmd)
        }
        CliCommand::HarnessSupport(args) => {
            if !FeatureFlag::AgentHarness.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'harness-support'"));
            }
            harness_support::run(ctx, global_options, args)
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
        CliCommand::Runner(runner_cmd) => {
            if !FeatureFlag::CloudAgentRunners.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'runner'"));
            }
            runner::run(ctx, global_options, runner_cmd)
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

/// Run the agent with the provided command.
fn run_agent(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: AgentCommand,
) -> anyhow::Result<()> {
    match command {
        AgentCommand::Run(args) => {
            if args.environment.is_some() && !FeatureFlag::CloudEnvironments.is_enabled() {
                return Err(anyhow::anyhow!("unexpected argument '--environment' found"));
            }
            if args.conversation.is_some() && !FeatureFlag::CloudConversations.is_enabled() {
                return Err(anyhow::anyhow!(
                    "unexpected argument '--conversation' found"
                ));
            }
            if args.skill.is_some() && !FeatureFlag::OzPlatformSkills.is_enabled() {
                return Err(anyhow::anyhow!("unexpected argument '--skill' found"));
            }
            if args.harness != Harness::Oz && !FeatureFlag::AgentHarness.is_enabled() {
                return Err(anyhow::anyhow!("unexpected argument '--harness' found"));
            }
            if args.harness == Harness::OpenCode {
                return Err(anyhow::anyhow!(
                    "The opencode harness is only supported for local child agent launches."
                ));
            }
            if args.harness_transport.is_some() && !FeatureFlag::AcpHarness.is_enabled() {
                return Err(anyhow::anyhow!(
                    "unexpected argument '--harness-transport' found"
                ));
            }

            let server_api = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();

            // Start the agent driver runner, which will handle the rest of the setup steps
            // (managing both sync and async steps) as well as triggering the driver.
            let runner = ctx.add_singleton_model(|_| AgentDriverRunner);
            runner.update(ctx, move |_, ctx| {
                let spawner = ctx.spawner();
                ctx.spawn(
                    AgentDriverRunner::setup_and_run_driver(
                        spawner,
                        args,
                        server_api,
                        global_options.output_format,
                    ),
                    |_, result, _ctx| {
                        if let Err(e) = result {
                            report_fatal_error(e.into(), _ctx);
                        }
                    },
                );
            });

            Ok(())
        }
        AgentCommand::RunCloud(args) => {
            if args.environment.environment.is_some()
                && !FeatureFlag::CloudEnvironments.is_enabled()
            {
                return Err(anyhow::anyhow!("unexpected argument '--environment' found"));
            }
            if args.conversation.is_some() && !FeatureFlag::CloudConversations.is_enabled() {
                return Err(anyhow::anyhow!(
                    "unexpected argument '--conversation' found"
                ));
            }
            if args.harness != Harness::Oz && !FeatureFlag::AgentHarness.is_enabled() {
                return Err(anyhow::anyhow!("unexpected argument '--harness' found"));
            }
            if let Err(msg) = args.validate_auth_secrets() {
                return Err(anyhow::anyhow!(msg));
            }
            if args.runner.is_some() && !FeatureFlag::CloudRunners.is_enabled() {
                return Err(anyhow::anyhow!("unexpected argument '--runner' found"));
            }
            ambient::run_ambient_agent(ctx, args)
        }
        AgentCommand::Profile(sub) => profiles::run(ctx, global_options, sub),
        AgentCommand::List(args) => {
            agent_management::list_agents(ctx, global_options.output_format, args)
        }
        AgentCommand::Get(args) => {
            agent_management::get_agent(ctx, global_options.output_format, args)
        }
        AgentCommand::Create(args) => {
            agent_management::create_agent(ctx, global_options.output_format, args)
        }
        AgentCommand::Update(args) => {
            agent_management::update_agent(ctx, global_options.output_format, args)
        }
        AgentCommand::Delete(args) => {
            agent_management::delete_agent(ctx, global_options.output_format, args)
        }
        AgentCommand::Skills(args) => agent_config::list_skills(ctx, args),
    }
}

/// Build the merged agent configuration from all sources and the Task for the driver.
/// Merge precedence: file < CLI < skill
fn build_merged_config_and_task(
    args: &RunAgentArgs,
    resolved_skill: &Option<ResolvedSkill>,
    prompt: &Option<Prompt>,
    local_run_team_scope: Option<&HeadlessTeamScope>,
    ctx: &mut AppContext,
) -> anyhow::Result<(AgentConfigSnapshot, Task)> {
    // Server-side prompt resolution (task_id is set): the task config already lives on the
    // server and individual CLI flags (--model, --mcp, etc.) are the only local overrides.
    // No config file is involved — the worker never passes --file alongside --task-id.
    if args.task_id.is_some() {
        return build_server_side_task(args, resolved_skill, ctx);
    }

    let loaded_file = match args.config_file.file.as_deref() {
        Some(path) => Some(config_file::load_config_file(path)?),
        None => None,
    };

    let cli_mcp_servers = build_mcp_servers_from_specs(&args.all_mcp_specs())?;

    // Merge precedence: file < CLI < skill
    let file_merged = config_file::merge_with_precedence(loaded_file.as_ref(), Default::default());

    // Runner support is gated. The `run` command has no `--runner` flag, but a
    // config file can still set `runner_id`, so reject it when the flag is off.
    if file_merged.runner_id.is_some() && !FeatureFlag::CloudRunners.is_enabled() {
        return Err(anyhow::anyhow!(
            "`runner_id` is set in the config file but runner support is not enabled"
        ));
    }

    // Skill provides base_prompt and optionally name
    let (skill_name, runtime_base_prompt) = match resolved_skill {
        Some(skill) => (Some(skill.name.clone()), Some(skill.instructions.clone())),
        None => (None, None),
    };

    // When a non-Oz harness is active, --model targets the harness rather than the Oz model.
    let harness_model_id = if args.harness != Harness::Oz {
        args.model.model.clone()
    } else {
        None
    };
    let harness_override = (args.harness != Harness::Oz).then_some(HarnessConfig {
        harness_type: args.harness,
        model_id: harness_model_id,
        reasoning_level: None,
    });

    let oz_model = if args.harness == Harness::Oz {
        args.model.model.clone().or(file_merged.model_id)
    } else {
        None
    };

    let mut merged_config = AgentConfigSnapshot {
        experimental: None,
        // CLI name > skill name > file name
        name: args.name.clone().or(skill_name).or(file_merged.name),
        environment_id: args.environment.clone().or(file_merged.environment_id),
        runner_id: file_merged.runner_id,
        model_id: oz_model,
        // Skill base_prompt takes precedence over file base_prompt
        base_prompt: runtime_base_prompt.clone().or(file_merged.base_prompt),
        mcp_servers: config_file::merge_mcp_servers(file_merged.mcp_servers, cli_mcp_servers),
        profile_id: args.profile.clone(),
        worker_host: file_merged.worker_host,
        skill_spec: file_merged.skill_spec,
        computer_use_enabled: args
            .computer_use
            .computer_use_override()
            .or(file_merged.computer_use_enabled),
        harness: harness_override,
        harness_auth_secrets: None,
        additional_source_repos: None,
    };

    let runtime_mcp_specs = match merged_config.mcp_servers.as_ref() {
        Some(mcp_servers) => config_file::mcp_specs_from_mcp_servers(mcp_servers)?,
        None => Vec::new(),
    };

    let model_override: Option<LLMId> = merged_config
        .model_id
        .as_deref()
        .filter(|_| args.harness == Harness::Oz)
        .map(|model_id| match local_run_team_scope {
            Some(team_scope) => {
                common::validate_agent_mode_base_model_id_for_scope(model_id, team_scope, ctx)
            }
            None => common::validate_agent_mode_base_model_id(model_id, ctx),
        })
        .transpose()?;

    // Keep the task config snapshot aligned with the effective model selection.
    merged_config.model_id = model_override.clone().map(|id| id.to_string());

    // Combine base_prompt with user prompt locally.
    let local_prompt = match (merged_config.base_prompt.as_deref(), prompt) {
        (Some(base_prompt), Some(Prompt::PlainText(user_prompt))) => {
            Prompt::PlainText(format!("{base_prompt}\n\n{user_prompt}"))
        }
        (Some(base_prompt), None) => {
            // Skill-only invocation: use skill instructions as the prompt
            Prompt::PlainText(base_prompt.to_string())
        }
        (_, Some(p)) => p.clone(),
        (None, None) => {
            return Err(anyhow::anyhow!(AgentDriverError::InvalidRuntimeState));
        }
    };

    let task = Task {
        prompt: AgentRunPrompt::Local(resolve_prompt(&local_prompt, ctx)?),
        model: model_override,
        profile: args.profile.clone(),
        mcp_specs: runtime_mcp_specs,
        harness: harness_kind(args.harness, args.harness_transport.unwrap_or_default())?,
    };

    Ok((merged_config, task))
}

/// Build the task for server-side prompt resolution (task_id is set).
/// Only CLI args contribute — no config file merge needed.
fn build_server_side_task(
    args: &RunAgentArgs,
    resolved_skill: &Option<ResolvedSkill>,
    ctx: &mut AppContext,
) -> anyhow::Result<(AgentConfigSnapshot, Task)> {
    let cli_mcp_servers = build_mcp_servers_from_specs(&args.all_mcp_specs())?;

    let runtime_mcp_specs = match cli_mcp_servers.as_ref() {
        Some(mcp_servers) => config_file::mcp_specs_from_mcp_servers(mcp_servers)?,
        None => Vec::new(),
    };

    let harness_model_id = if args.harness != Harness::Oz {
        args.model.model.clone()
    } else {
        None
    };
    let model_override: Option<LLMId> = if args.harness == Harness::Oz {
        args.model
            .model
            .as_deref()
            .map(|model_id| common::validate_agent_mode_base_model_id(model_id, ctx))
            .transpose()?
    } else {
        None
    };

    let harness_override = (args.harness != Harness::Oz).then_some(HarnessConfig {
        harness_type: args.harness,
        model_id: harness_model_id,
        reasoning_level: None,
    });

    let skill_name = resolved_skill.as_ref().map(|s| s.name.clone());
    let model_id_string = model_override.as_ref().map(|id| id.to_string());
    let profile = args.profile.clone();
    let environment = args.environment.clone();

    let config = AgentConfigSnapshot {
        experimental: None,
        name: args.name.clone().or(skill_name),
        environment_id: environment.clone(),
        runner_id: None,
        model_id: model_id_string,
        base_prompt: None,
        mcp_servers: cli_mcp_servers,
        profile_id: profile.clone(),
        worker_host: None,
        skill_spec: None,
        computer_use_enabled: args.computer_use.computer_use_override(),
        harness: harness_override,
        harness_auth_secrets: None,
        additional_source_repos: None,
    };

    let skill = resolved_skill.as_ref().map(|s| s.parsed_skill.clone());

    let task = Task {
        prompt: AgentRunPrompt::ServerSide {
            skill,
            attachments_dir: None,
        },
        model: model_override,
        profile,
        mcp_specs: runtime_mcp_specs,
        harness: harness_kind(args.harness, args.harness_transport.unwrap_or_default())?,
    };

    Ok((config, task))
}

fn build_execution_task_and_options(
    args: &RunAgentArgs,
    config: ExecutionConfiguration,
    working_dir: PathBuf,
    first_skill: Option<ResolvedSkill>,
    skill_discovery_dirs: Vec<PathBuf>,
    ctx: &mut AppContext,
) -> anyhow::Result<(
    AgentDriverOptions,
    Task,
    Option<String>,
    crate::ai::cloud_environments::ProvidersConfig,
)> {
    let task_id = args
        .task_id
        .as_deref()
        .context("Execution configuration requires a task ID")?
        .parse::<AmbientAgentTaskId>()
        .context("Invalid execution task ID")?;
    let selected_harness = execution_config::harness(&config.harness)?;
    let model = if selected_harness == Harness::Oz {
        config
            .model_id
            .as_deref()
            .map(|id| common::validate_agent_mode_base_model_id(id, ctx))
            .transpose()?
    } else {
        None
    };
    let third_party_harness_model_config = (selected_harness != Harness::Oz)
        .then_some(HarnessConfig {
            harness_type: selected_harness,
            model_id: config.model_id,
            reasoning_level: config.reasoning_level,
        })
        .and_then(|config| config.model_config());
    let mcp_specs = execution_config::mcp_specs(&config.mcp_servers_json)?;
    let repositories = execution_config::repositories(config.repositories)?;
    let idle_on_complete = execution_config::idle_duration(config.idle_on_complete_seconds)?;
    let idle_on_fail = execution_config::idle_duration(config.idle_on_fail_seconds)?;
    if config.skip_initial_turn && idle_on_complete.is_none() {
        anyhow::bail!("An execution that skips its initial turn requires an idle window");
    }
    let providers = execution_config::providers(config.providers);
    let mut factory_skill_dirs = execution_config::factory_skill_dirs(config.factory_skill_dirs)?;
    for dir in skill_discovery_dirs {
        if !factory_skill_dirs.contains(&dir) {
            factory_skill_dirs.push(dir);
        }
    }
    let task = Task {
        prompt: AgentRunPrompt::ServerSide {
            skill: first_skill.map(|skill| skill.parsed_skill),
            attachments_dir: None,
        },
        model,
        profile: config.profile_id.map(|id| id.into_inner()),
        mcp_specs,
        harness: harness_kind(selected_harness, args.harness_transport.unwrap_or_default())?,
    };
    let options = AgentDriverOptions {
        working_dir,
        task_id: Some(task_id),
        experimental: None,
        parent_run_id: config.parent_run_id.map(|id| id.into_inner()),
        should_share: FeatureFlag::AgentSharedSessions.is_enabled(),
        idle_on_complete,
        idle_on_fail,
        secrets: Default::default(),
        resume: None,
        cloud_providers: Vec::new(),
        workspace: {
            let mut workspace = driver::environment::WorkspaceConfiguration::from_resolved(
                repositories,
                config.setup_commands,
            )?;
            workspace.factory_skill_dirs = Some(factory_skill_dirs);
            workspace
        },
        computer_use_config: (selected_harness == Harness::Oz).then(|| {
            (
                config.computer_use_enabled,
                config.computer_use_model_id.map(Into::into),
            )
        }),
        selected_harness,
        harness_transport: args.harness_transport.unwrap_or_default(),
        third_party_harness_model_config,
        team_scope: None,
        bedrock_oidc_credentials: None,
        snapshot_disabled: Some(config.snapshot_disabled),
        snapshot_upload_timeout: args.snapshot.snapshot_upload_timeout.map(Into::into),
        snapshot_script_timeout: args.snapshot.snapshot_script_timeout.map(Into::into),
        checkpoint_interval: None,
        skip_initial_turn: config.skip_initial_turn,
        strict_mcp_startup: args.strict_mcp_startup,
        mcp_startup_timeout: args.mcp_startup_timeout.map(Into::into),
    };
    Ok((
        options,
        task,
        config.conversation_id.map(|id| id.into_inner()),
        providers,
    ))
}
fn selected_execution_skills(
    skills: Vec<ResolvedSkill>,
) -> Result<(Option<ResolvedSkill>, Vec<PathBuf>), AgentDriverError> {
    let mut first = None;
    let mut discovery_dirs = Vec::new();
    for skill in skills {
        let skills_dir = skill
            .skill_path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| {
                AgentDriverError::SkillResolutionFailed(
                    "Resolved skill has no parent skill directory".to_owned(),
                )
            })?
            .to_path_buf();
        if skills_dir.to_string_lossy().contains(',') {
            return Err(AgentDriverError::SkillResolutionFailed(
                "Resolved skill directory contains a comma".to_owned(),
            ));
        }
        if !discovery_dirs.contains(&skills_dir) {
            discovery_dirs.push(skills_dir);
        }
        if first.is_none() {
            first = Some(skill);
        }
    }
    Ok((first, discovery_dirs))
}

fn reconcile_task_harness(
    task_id: &str,
    selected_harness: &mut Harness,
    task_harness: Harness,
    transport: HarnessTransport,
) -> Result<HarnessKind, AgentDriverError> {
    if *selected_harness == Harness::Oz {
        *selected_harness = task_harness;
    } else if task_harness != *selected_harness {
        return Err(AgentDriverError::TaskHarnessMismatch {
            task_id: task_id.to_string(),
            expected: task_harness.to_string(),
            got: selected_harness.to_string(),
        });
    }

    harness_kind(*selected_harness, transport)
}

/// Resolve a `Prompt` to a plain string.
fn resolve_prompt(prompt: &Prompt, ctx: &AppContext) -> Result<String, AgentDriverError> {
    match prompt {
        Prompt::PlainText(prompt_str) => Ok(prompt_str.to_string()),
        Prompt::SavedPrompt(workflow_id) => {
            let Some(workflow) = CloudModel::as_ref(ctx).get_workflow_by_uid(workflow_id) else {
                return Err(AgentDriverError::AIWorkflowNotFound(workflow_id.to_owned()));
            };

            let Workflow::AgentMode { query, .. } = &workflow.model().data else {
                return Err(AgentDriverError::AIWorkflowNotFound(workflow_id.to_owned()));
            };
            Ok(query.to_owned())
        }
    }
}

/// Run the task with the provided command.
fn run_task(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: TaskCommand,
) -> anyhow::Result<()> {
    match command {
        TaskCommand::List(args) => ambient::list_ambient_agent_tasks(ctx, global_options, args),
        TaskCommand::Get(args) => {
            if args.conversation {
                if !FeatureFlag::ConversationApi.is_enabled() {
                    return Err(anyhow::anyhow!(
                        "The --conversation flag is not available in this build"
                    ));
                }
                ambient::get_run_conversation(ctx, args.task_id)
            } else {
                ambient::get_ambient_agent_task_status(ctx, global_options, args)
            }
        }
        TaskCommand::Conversation(conv_cmd) => {
            if !FeatureFlag::ConversationApi.is_enabled() {
                return Err(anyhow::anyhow!(
                    "The 'conversation' subcommand is not available in this build"
                ));
            }
            match conv_cmd {
                warp_cli::task::ConversationCommand::Get(args) => {
                    ambient::get_conversation(ctx, args.conversation_id)
                }
            }
        }
        TaskCommand::Message(message_cmd) => ambient::run_message(ctx, global_options, message_cmd),
    }
}

/// Singleton model that provides a ModelContext for spawning async operations
/// when starting the agent driver. This is needed because conversation fetching
/// requires spawning an async task, which requires a ModelContext.
struct AgentDriverRunner;
struct ExecutionTaskData {
    server_api: Arc<ServerApi>,
    config: ExecutionConfiguration,
    secrets: HashMap<String, ManagedSecretValue>,
    attachments: anyhow::Result<Vec<TaskAttachment>>,
}

impl warpui::Entity for AgentDriverRunner {
    type Event = ();
}

impl warpui::SingletonEntity for AgentDriverRunner {}

fn resolve_agent_driver_team_scope(
    args: &RunAgentArgs,
    ctx: &AppContext,
) -> anyhow::Result<Option<HeadlessTeamScope>> {
    // We need a team scope if either:
    // 1. This is a local team-visible task that doesn't exist on the server yet.
    // 2. This task is authenticated as a service account
    if args.task_id.is_some() && !AuthStateProvider::as_ref(ctx).get().is_service_account() {
        return Ok(None);
    }
    let scope = common::resolve_team_scope(&args.team_selection, ctx)?;
    Ok(Some(scope))
}

impl AgentDriverRunner {
    #[tracing::instrument(skip_all, err, fields(
        tags.cloud_agent = true,
        args.sandboxed = args.sandboxed,
        args.computer_use = args.computer_use.computer_use,
        args.no_computer_use = args.computer_use.no_computer_use
    ))]
    async fn setup_and_run_driver(
        foreground: ModelSpawner<Self>,
        args: RunAgentArgs,
        server_api: Arc<dyn AIClient>,
        output_format: OutputFormat,
    ) -> Result<(), AgentDriverError> {
        // Extract the task ID as early as possible for best-effort setup observability.
        // Local CLI-created runs may not have a task yet, so those setup events explicitly no-op.
        let mut task_id: Option<AmbientAgentTaskId> =
            args.task_id.as_deref().and_then(|s| s.parse().ok());
        // Set up and run the driver, reporting any errors back to the server.
        let result: Result<(), AgentDriverError> = async {
            Self::set_ambient_agent_task_id(&foreground, task_id).await?;
            let background = foreground.spawn(|_, ctx| ctx.background_executor()).await?;
            let setup_events = match task_id {
                Some(task_id) => SetupClientEventReporter::new(task_id, server_api.clone(), background),
                None => SetupClientEventReporter::noop(server_api.clone(), background),
            };
            setup_events
                .post_timeline_event(OzRunTimelineEvent::WorkerContainerReady)
                .await;
            let execution_data = if FeatureFlag::CloudAgentExecutionConfig.is_enabled()
                && let (Some(task_id), Some(execution_id)) =
                    (args.task_id.as_deref(), args.execution_id.as_deref())
            {
                Some(Self::fetch_execution_task_data(&foreground, task_id, execution_id).await?)
            } else {
                None
            };
            let execution_config = execution_data.as_ref().map(|data| &data.config);
            let has_execution_config = execution_config.is_some();

            // Ensure we've synced team state before starting the driver.
            setup_events
                .record_result(
                    SetupStep::TeamMetadataRefresh,
                    Self::refresh_team_metadata(&foreground),
                )
                .await?;
            let agent_driver_team_scope = if let Some(config) = execution_config {
                Some(execution_config::team_scope(config.team_id.as_ref())
                    .map_err(AgentDriverError::ConfigBuildFailed)?)
            } else {
                let args_for_team_scope = args.clone();
                foreground
                    .spawn(move |_, ctx| resolve_agent_driver_team_scope(&args_for_team_scope, ctx))
                    .await?
                    .map_err(AgentDriverError::ConfigBuildFailed)?
            };

            // Wait for Warp Drive to sync before building the task config, since
            // prompt resolution (SavedPrompt -> workflow lookup) and environment
            // resolution (CloudAmbientAgentEnvironment lookup) depend on it.
            setup_events
                .record_result(SetupStep::WarpDriveSync, async {
                    if foreground
                        .spawn(|_, ctx| common::refresh_warp_drive(ctx))
                        .await?
                        .await
                        .is_err()
                    {
                        return Err(AgentDriverError::WarpDriveSyncFailed);
                    }
                    Ok(())
                })
                .await?;

            // Pull relevant variables out of args before moving it into the closure.
            let share_requests = match execution_config {
                Some(config) => Some(execution_config::sharing_acls(config.session_sharing_acls.clone())
                    .map_err(AgentDriverError::ConfigBuildFailed)?),
                None => args.share.share.clone(),
            };
            let bedrock_inference_role = match execution_config {
                Some(config) => config.inference_providers.as_ref()
                    .and_then(|providers| providers.aws_bedrock.as_ref())
                    .map(|bedrock| bedrock.role_arn.clone()),
                None => args.bedrock_inference_role.clone(),
            };
            let bedrock_role_region = match execution_config {
                Some(config) => config.inference_providers.as_ref()
                    .and_then(|providers| providers.aws_bedrock.as_ref())
                    .and_then(|bedrock| bedrock.region.clone()),
                None => args.bedrock_role_region.clone(),
            };
            let has_task_id = args.task_id.is_some();
            let resume_conversation_id = if has_execution_config {
                None
            } else {
                args.conversation.clone()
            };

            // Build driver options and task, handling task creation or existing task setup.
            // For the `--task-id` path, `task_conversation_id` is the `conversation_id` read off
            // the fetched `AmbientAgentTask` (set by the server when linking the task to an
            // existing conversation, e.g. via `run-cloud --conversation`).
            let (mut driver_options, task, task_conversation_id) =
                Self::build_driver_options_and_task(
                    &foreground,
                    args,
                    agent_driver_team_scope,
                    &server_api,
                    &setup_events,
                    execution_data,
                )
                .await?;

            // Update the effective task ID so errors are reported correctly.
            // This only matters if we created a task ID locally.
            task_id = driver_options.task_id.or(task_id);

            let resume_conversation_id = resume_conversation_id.or(task_conversation_id);
            if (has_execution_config || !has_task_id)
                && let Some(conversation_id) = resume_conversation_id.as_deref()
            {
                common::fetch_and_validate_conversation_harness(
                    server_api.clone(),
                    conversation_id,
                    task.harness.harness(),
                )
                .await?;
            }

            #[cfg(not(target_family = "wasm"))]
            if let Some(role_arn) = bedrock_inference_role {
                // clap's `requires` constraint enforces this at parse time, so a missing
                // region here means a caller is constructing `RunAgentArgs` directly
                // without the flag. Fail loudly so callers don't silently fall back to a
                // hard-coded STS region.
                let role_region = bedrock_role_region.ok_or_else(|| {
                    AgentDriverError::AwsBedrockCredentialsFailed(
                        "--bedrock-role-region is required when --bedrock-inference-role is set"
                            .to_string(),
                    )
                })?;
                let config =
                    bedrock_oidc_credentials_config(&driver_options, role_arn, role_region)?;
                let request_scope = driver_options
                    .team_scope
                    .as_ref()
                    .map(RequestTeamScope::from_scope);
                driver_options.bedrock_oidc_credentials = Some(config.clone());
                // Set the OIDC strategy on the UI thread and kick off the refresh; the
                // returned future resolves when credentials are committed to the model.
                let refresh_future = foreground
                    .spawn(move |_, ctx| {
                        ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                            // From here on, refresh credentials via OIDC federation only.
                            manager.set_aws_credentials_refresh_strategy(
                                AwsCredentialsRefreshStrategy::OidcManaged,
                                ctx,
                            );
                            refresh_aws_credentials_oidc(config, request_scope, manager, ctx)
                        })
                    })
                    .await?;

                refresh_future
                    .await
                    .map_err(AgentDriverError::AwsBedrockCredentialsFailed)?;
            }

            match &task.harness {
                HarnessKind::Unsupported(harness) => {
                    return Err(AgentDriverError::HarnessSetupFailed {
                        harness: harness.to_string(),
                        reason: format!(
                            "The {harness} harness is only supported for local child agent launches."
                        ),
                    });
                }
                HarnessKind::Oz | HarnessKind::ThirdParty(_) => {}
            }

            // Validate that the third-party harness is installed and authed.
            if let HarnessKind::ThirdParty(harness) = &task.harness {
                harness.validate()?;
            }

            if let Some(task_id) = driver_options.task_id {
                driver::write_run_started(&task_id.to_string(), output_format);
            }

            // Pull conversation information, if we have it
            if let Some(conversation_id) = resume_conversation_id {
                driver_options.resume = setup_events
                    .record_result(
                        SetupStep::ConversationResumeLoading,
                        Self::load_conversation_information(
                            &foreground,
                            conversation_id,
                            &task.harness,
                        ),
                    )
                    .await?;
            }

            // Run the driver
            foreground
                .spawn(move |_, ctx| {
                    Self::create_and_run_driver(
                        ctx,
                        driver_options,
                        output_format,
                        share_requests,
                        task,
                    );
                })
                .await?;

            Ok(())
        }
        .await;

        if let Err(ref err) = result
            && let Some(task_id) = task_id
        {
            driver::report_driver_error(task_id, err, &server_api).await;
        }
        result
    }

    async fn refresh_team_metadata(
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        foreground
            .spawn(
                |_, ctx| -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
                    Box::pin(common::refresh_workspace_metadata(ctx))
                },
            )
            .await?
            .await
            .map_err(AgentDriverError::TeamMetadataRefreshFailed)
    }

    async fn set_ambient_agent_task_id(
        foreground: &ModelSpawner<Self>,
        task_id: Option<AmbientAgentTaskId>,
    ) -> Result<(), AgentDriverError> {
        #[cfg(feature = "crash_reporting")]
        if let Some(task_id) = task_id {
            crate::crash_reporting::set_task_id_tag(&task_id.to_string());
        }
        foreground
            .spawn(move |_, ctx| {
                ServerApiProvider::handle(ctx)
                    .as_ref(ctx)
                    .get()
                    .set_ambient_agent_task_id(task_id);
            })
            .await?;
        Ok(())
    }

    async fn fetch_execution_task_data(
        foreground: &ModelSpawner<Self>,
        task_id: &str,
        execution_id: &str,
    ) -> Result<ExecutionTaskData, AgentDriverError> {
        let (workload_token, no_isolation) =
            match warp_isolation_platform::issue_workload_token(Some(Duration::from_mins(5))).await
            {
                Ok(token) => (token.token, false),
                Err(IsolationPlatformError::NoIsolationPlatformDetected) => (String::new(), true),
                Err(error) => return Err(AgentDriverError::SecretsFetchFailed(error.into())),
            };
        let api = foreground
            .spawn(|_, ctx| ServerApiProvider::as_ref(ctx).get())
            .await?;
        let data = with_retry(
            "Execution bootstrap",
            || api.get_execution_bootstrap(task_id, execution_id, workload_token.clone()),
            retry::is_transient_graphql_or_http_error,
            |delay| async move {
                warpui::r#async::Timer::after(delay).await;
            },
            |attempts_made| match attempts_made {
                0 => Some(Duration::from_millis(250)),
                1 => Some(Duration::from_secs(1)),
                _ => None,
            },
        )
        .await
        .map_err(AgentDriverError::ConfigBuildFailed)?;
        let secrets = if no_isolation {
            HashMap::new()
        } else {
            convert_task_secrets(data.secrets.map_err(AgentDriverError::SecretsFetchFailed)?)
                .map_err(AgentDriverError::SecretsFetchFailed)?
        };
        Ok(ExecutionTaskData {
            server_api: api,
            config: data.config,
            secrets,
            attachments: data.attachments,
        })
    }

    async fn fetch_task_git_credentials(
        task_id_str: String,
        ai_client: Arc<dyn AIClient>,
    ) -> Result<Vec<GitCredential>, TaskGitCredentialsError> {
        with_retry(
            "Git credentials bootstrap",
            || {
                let task_id_str = task_id_str.clone();
                let ai_client = Arc::clone(&ai_client);
                async move {
                    driver::git_credentials::ensure_workload_token_available()?;

                    let workload_token = warp_isolation_platform::issue_workload_token(Some(
                        std::time::Duration::from_secs(5 * 60),
                    ))
                    .await
                    .map_err(|error| TaskGitCredentialsError::Request(error.into()))?
                    .token;
                    let response = ai_client
                        .get_task_git_credentials(task_id_str, workload_token, false)
                        .await?;
                    driver::git_credentials::credentials_for_bootstrap(response)
                        .map_err(TaskGitCredentialsError::Request)
                }
            },
            driver::git_credentials::is_retryable,
            |delay| async move {
                warpui::r#async::Timer::after(delay).await;
            },
            |attempts_made| {
                driver::git_credentials::GIT_CREDENTIALS_BOOTSTRAP_BACKOFF
                    .get(attempts_made)
                    .copied()
            },
        )
        .await
    }

    async fn bootstrap_git_credentials_for_task(
        foreground: &ModelSpawner<Self>,
        task_id_str: &str,
        args: &RunAgentArgs,
    ) -> Result<(), AgentDriverError> {
        // The gh CLI only covers github.com, so this must not replace the
        // server fetch below — other forges (GitLab, Azure DevOps) get their
        // credentials exclusively from the server.
        let git_credentials_configured_with_gh = warp_isolation_platform::detect().is_none()
            && args.configure_git_credentials_with_github;
        if git_credentials_configured_with_gh {
            foreground
                .spawn(|_, _| {
                    command::blocking::Command::new("gh")
                        .args(["auth", "setup-git"])
                        .spawn()
                        .map_err(|err| {
                            AgentDriverError::ConfigBuildFailed(anyhow::anyhow!(
                                "gh auth setup-git failed: {err:?}"
                            ))
                        })
                })
                .await?
                .map(|_| ())?;
        }

        if !FeatureFlag::GitCredentialRefresh.is_enabled() {
            return Ok(());
        }

        if task_id_str.parse::<AmbientAgentTaskId>().is_err() {
            log::debug!(
                "Skipping git credentials bootstrap: could not parse task ID '{task_id_str}'"
            );
            return Ok(());
        }

        let (ai_client, task_id_str) = foreground
            .spawn({
                let task_id_str = task_id_str.to_string();
                move |_, ctx| {
                    let ai_client = ServerApiProvider::handle(ctx)
                        .as_ref(ctx)
                        .get_ai_client()
                        .clone();
                    (ai_client, task_id_str)
                }
            })
            .await?;

        let credentials = match Self::fetch_task_git_credentials(
            task_id_str.clone(),
            Arc::clone(&ai_client),
        )
        .await
        {
            Ok(credentials) => credentials,
            Err(TaskGitCredentialsError::Request(err))
                if err
                    .downcast_ref::<IsolationPlatformError>()
                    .is_some_and(|err| {
                        matches!(err, IsolationPlatformError::NoIsolationPlatformDetected)
                    }) =>
            {
                log::debug!("Skipping git credentials bootstrap: {err}");
                return Ok(());
            }
            Err(err) if git_credentials_configured_with_gh => {
                // gh already configured github.com above, so a failed server fetch degrades to
                // GitHub-only credentials instead of failing a run that previously worked without
                // the fetch.
                log::warn!(
                    "Failed to fetch git credentials; continuing with gh-configured GitHub credentials only: {err:#}"
                );
                return Ok(());
            }
            Err(err) => {
                log::warn!("Failed to fetch git credentials before skill resolution: {err:#}");
                tracing::warn!(
                    error = ?err,
                    "Failed to fetch git credentials before skill resolution"
                );
                return Err(AgentDriverError::GitCredentialsFetchFailed(err));
            }
        };
        if credentials.is_empty() {
            log::debug!("No git credentials returned before skill resolution");
            return Ok(());
        }

        driver::git_credentials::configure_git_credentials(&credentials).map_err(|err| {
            log::warn!("Failed to write git credentials before skill resolution: {err:#}");
            tracing::warn!(
                error = ?err,
                "Failed to write git credentials before skill resolution"
            );
            AgentDriverError::SkillResolutionFailed(format!(
                "Failed to write git credentials before skill resolution: {err:#}"
            ))
        })?;
        log::info!("Git credentials configured before task setup");
        Ok(())
    }

    /// Resolve the skill spec from args, if one was provided.
    ///
    /// In sandboxed mode with a fully-qualified spec (org + repo), the repo is
    /// cloned first since it may not exist locally. Otherwise we resolve directly
    /// against the local filesystem.
    async fn resolve_skill(
        foreground: &ModelSpawner<Self>,
        args: &RunAgentArgs,
        working_dir: &Path,
        setup_events: &SetupClientEventReporter,
    ) -> Result<Option<ResolvedSkill>, AgentDriverError> {
        if !FeatureFlag::OzPlatformSkills.is_enabled() {
            return Ok(None);
        }
        let Some(skill_spec) = args.skill.clone() else {
            return Ok(None);
        };

        // In sandboxed mode with a fully-qualified spec, clone the repo first.
        let needs_clone = args.sandboxed && skill_spec.org.is_some() && skill_spec.repo.is_some();
        if needs_clone {
            let org = skill_spec.org.as_ref().expect("org checked above");
            let repo_name = skill_spec.repo.as_ref().expect("repo checked above");
            log::info!("Cloning {org}/{repo_name} for skill resolution in sandboxed mode");
            setup_events
                .record_result(SetupStep::SkillRepoClone, async {
                    clone_repo_for_skill(org, repo_name, working_dir)
                        .await
                        .map_err(|err| {
                            AgentDriverError::SkillResolutionFailed(format_skill_resolution_error(
                                err,
                            ))
                        })
                })
                .await?;
        }

        let working_dir_buf = working_dir.to_path_buf();
        let skill = foreground
            .spawn(move |_, ctx| resolve_skill_spec(&skill_spec, &working_dir_buf, ctx))
            .await?
            .map_err(|err| {
                AgentDriverError::SkillResolutionFailed(format_skill_resolution_error(err))
            })?;
        log::debug!(
            "Resolved skill '{}' from {}",
            skill.name,
            skill.skill_path.display()
        );
        Ok(Some(skill))
    }

    async fn resolve_execution_skills(
        foreground: &ModelSpawner<Self>,
        args: &RunAgentArgs,
        config: &ExecutionConfiguration,
        working_dir: &Path,
        setup_events: &SetupClientEventReporter,
    ) -> Result<(Option<ResolvedSkill>, Vec<PathBuf>), AgentDriverError> {
        if !config.skills.is_empty() && !FeatureFlag::OzPlatformSkills.is_enabled() {
            return Err(AgentDriverError::SkillResolutionFailed(
                "Skills are not supported in this client build".to_owned(),
            ));
        }
        let mut resolved = Vec::with_capacity(config.skills.len());
        for entry in &config.skills {
            let spec: SkillSpec = entry
                .spec
                .parse()
                .map_err(|error: String| AgentDriverError::SkillResolutionFailed(error))?;
            let mut skill_args = args.clone();
            skill_args.skill = Some(spec);
            let skill = Self::resolve_skill(foreground, &skill_args, working_dir, setup_events)
                .await?
                .ok_or(AgentDriverError::InvalidRuntimeState)?;
            resolved.push(skill);
        }
        selected_execution_skills(resolved)
    }

    /// Build the AgentDriverOptions and Task, handling task creation or existing task setup.
    ///
    /// The third tuple element is the conversation id read off the server-side task metadata
    /// on the `--task-id` branch. It's `None` when no task id was passed or when the task is
    /// not linked to a conversation; callers use it to drive `--task-id`-implied resume
    /// without requiring the caller to also pass `--conversation`.
    async fn build_driver_options_and_task(
        foreground: &ModelSpawner<Self>,
        args: RunAgentArgs,
        agent_driver_team_scope: Option<HeadlessTeamScope>,
        server_api: &Arc<dyn AIClient>,
        setup_events: &SetupClientEventReporter,
        execution_data: Option<ExecutionTaskData>,
    ) -> Result<(AgentDriverOptions, Task, Option<String>), AgentDriverError> {
        // Get the working directory
        let working_dir = match args.cwd.as_ref() {
            Some(dir) => dunce::canonicalize(dir)
                .with_context(|| format!("Unable to resolve {}", dir.display())),
            None => std::env::current_dir().context("Unable to determine working directory"),
        }
        .map_err(AgentDriverError::ConfigBuildFailed)?;

        if let Some(task_id_str) = args.task_id.as_ref() {
            Self::bootstrap_git_credentials_for_task(foreground, task_id_str, &args).await?;
        }
        if let Some(ExecutionTaskData {
            server_api: execution_server_api,
            config,
            secrets,
            attachments,
        }) = execution_data
        {
            let (first_skill, skill_discovery_dirs) = Self::resolve_execution_skills(
                foreground,
                &args,
                &config,
                &working_dir,
                setup_events,
            )
            .await?;
            let (mut options, mut task, conversation_id, providers) = foreground
                .spawn(move |_, ctx| {
                    build_execution_task_and_options(
                        &args,
                        config,
                        working_dir,
                        first_skill,
                        skill_discovery_dirs,
                        ctx,
                    )
                })
                .await?
                .map_err(AgentDriverError::ConfigBuildFailed)?;
            options.team_scope = agent_driver_team_scope;
            let task_id = options
                .task_id
                .ok_or(AgentDriverError::InvalidRuntimeState)?;
            setup_events
                .record_result(
                    SetupStep::TaskDataFetch,
                    Self::prepare_execution_task_data(
                        execution_server_api,
                        server_api,
                        task_id,
                        secrets,
                        attachments,
                        &mut options,
                        &mut task,
                    ),
                )
                .await?;
            if FeatureFlag::OzIdentityFederation.is_enabled() {
                options.cloud_providers = driver::cloud_provider::load_providers(
                    &providers,
                    &options
                        .task_id
                        .ok_or(AgentDriverError::InvalidRuntimeState)?
                        .to_string(),
                )
                .map_err(AgentDriverError::CloudProviderSetupFailed)?;
            }
            return Ok((options, task, conversation_id));
        }
        // Resolve the skill, if we have one
        let resolved_skill =
            Self::resolve_skill(foreground, &args, &working_dir, setup_events).await?;

        // Extract variables we want to use later before moving args into the closure
        let task_id_str = args.task_id.clone();
        let prompt = args.prompt_arg.to_prompt();
        let skill = args.skill.clone();

        // Build the AgentConfigSnapshot, Task, and AgentDriverOptions
        let prompt_clone = prompt.clone();
        let (
            merged_config,
            mut task,
            mut driver_options,
            agent_driver_team_scope,
            repository_preparation_overrides,
            remove_repository_origins,
        ) = foreground
            .spawn(move |_, ctx| -> anyhow::Result<_> {
                let (merged_config, task) = build_merged_config_and_task(
                    &args,
                    &resolved_skill,
                    &prompt_clone,
                    agent_driver_team_scope.as_ref(),
                    ctx,
                )?;

                let task_id = args.task_id.as_ref().and_then(|s| s.parse().ok());
                let should_share = (args.share.is_shared() || args.task_id.is_some())
                    && FeatureFlag::AgentSharedSessions.is_enabled();

                let third_party_harness_model_config = merged_config
                    .harness
                    .as_ref()
                    .and_then(|h| h.model_config());
                let driver_options = driver::AgentDriverOptions {
                    working_dir: working_dir.clone(),
                    task_id,
                    experimental: None,
                    parent_run_id: None,
                    should_share,
                    idle_on_complete: args.idle_on_complete.map(|d| d.into()),
                    idle_on_fail: args.effective_idle_on_fail().map(|d| d.into()),
                    secrets: Default::default(),
                    resume: None,
                    cloud_providers: Vec::new(),
                    workspace: driver::environment::WorkspaceConfiguration::default(),
                    computer_use_config: None,
                    selected_harness: args.harness,
                    harness_transport: args.harness_transport.unwrap_or_default(),
                    third_party_harness_model_config,
                    team_scope: None,
                    bedrock_oidc_credentials: None,
                    snapshot_disabled: args.snapshot.no_snapshot.then_some(true),
                    snapshot_upload_timeout: args
                        .snapshot
                        .snapshot_upload_timeout
                        .map(|duration| duration.into()),
                    snapshot_script_timeout: args
                        .snapshot
                        .snapshot_script_timeout
                        .map(|duration| duration.into()),
                    checkpoint_interval: None,
                    skip_initial_turn: args.skip_initial_turn,
                    strict_mcp_startup: args.strict_mcp_startup,
                    mcp_startup_timeout: args.mcp_startup_timeout.map(|duration| duration.into()),
                };

                Ok((
                    merged_config,
                    task,
                    driver_options,
                    agent_driver_team_scope,
                    args.repository_preparation_overrides,
                    args.remove_repository_origins,
                ))
            })
            .await?
            .map_err(AgentDriverError::ConfigBuildFailed)?;

        let environment_id = merged_config.environment_id.clone();

        // Handle secrets/attachments fetch (existing task) or task creation (new run).
        // The existing-task branch also surfaces the task's `conversation_id` (if any) so
        // the caller can wire up resume without a separate `--conversation` arg.
        let (task_conversation_id, additional_source_repos) = if let Some(task_id_str) = task_id_str
        {
            driver_options.team_scope = agent_driver_team_scope;
            setup_events
                .record_result(
                    SetupStep::TaskDataFetch,
                    Self::fetch_secrets_and_attachments(
                        foreground,
                        server_api,
                        task_id_str,
                        &mut driver_options,
                        &mut task,
                    ),
                )
                .await?
        } else {
            // Extract the prompt text that we'll pass up to the server when we create the task.
            let prompt_for_task_creation = match &prompt {
                Some(Prompt::PlainText(text)) => text.clone(),
                Some(Prompt::SavedPrompt(id)) => format!("Saved prompt ({id})"),
                None => skill
                    .as_ref()
                    .map(|s| format!("/{}", s.skill_identifier))
                    // If we get to this point and we don't have a prompt, saved prompt, or skill,
                    // error. `clap` should have handled this when parsing args already.
                    .ok_or(AgentDriverError::InvalidRuntimeState)?,
            };

            Self::initialize_new_task(
                foreground,
                server_api,
                prompt_for_task_creation,
                merged_config,
                agent_driver_team_scope.expect("new local runs resolve a team scope"),
                &mut driver_options,
            )
            .await?;
            (None, Vec::new())
        };
        // Resolve environment and cloud providers.
        let environment = setup_events
            .record_result(
                SetupStep::EnvironmentResolution,
                Self::resolve_environment(foreground, environment_id, &mut driver_options),
            )
            .await?;
        driver_options.workspace = driver::environment::WorkspaceConfiguration::from_legacy(
            environment.as_ref(),
            additional_source_repos,
            repository_preparation_overrides,
            remove_repository_origins,
        )?;

        Ok((driver_options, task, task_conversation_id))
    }

    /// Creates a new task on the server for this agent run, sets the task ID on the driver
    /// options, and updates the Server API provider so that all subsequent requests to warp-server
    /// contain this new task ID.
    async fn initialize_new_task(
        foreground: &ModelSpawner<Self>,
        server_api: &Arc<dyn AIClient>,
        prompt: String,
        merged_config: AgentConfigSnapshot,
        team_scope: HeadlessTeamScope,
        driver_options: &mut AgentDriverOptions,
    ) -> Result<(), AgentDriverError> {
        let request_team_scope = RequestTeamScope::from_scope(&team_scope);
        driver_options.team_scope = Some(team_scope);
        let environment = merged_config.environment_id.clone();
        let task_config = if merged_config.is_empty() {
            None
        } else {
            let mut config = merged_config;
            // We don't set a worker, since this is a local run.
            config.worker_host = None;
            Some(config)
        };

        let task_id = match server_api
            .create_agent_task(prompt, environment, None, task_config, request_team_scope)
            .await
            .context("Failed to create task")
        {
            Ok(id) => {
                log::info!("Created task: {id}");
                Some(id)
            }
            Err(e) => {
                report_error!(e);
                // Continue without a task_id rather than failing entirely
                None
            }
        };

        // Set the task ID on the ServerApi so it's sent with all subsequent requests.
        Self::set_ambient_agent_task_id(foreground, task_id).await?;
        driver_options.task_id = task_id;

        Ok(())
    }

    /// Downloads regular and handoff attachments independently before the run begins.
    async fn prepare_execution_task_data(
        server_api: Arc<ServerApi>,
        ai_client: &Arc<dyn AIClient>,
        task_id: AmbientAgentTaskId,
        secrets: HashMap<String, ManagedSecretValue>,
        attachments: anyhow::Result<Vec<TaskAttachment>>,
        driver_options: &mut AgentDriverOptions,
        task: &mut Task,
    ) -> Result<(), AgentDriverError> {
        let attachments_dir = attachments_download_dir(&driver_options.working_dir);
        let regular = async {
            match attachments {
                Ok(attachments) => {
                    driver::attachments::download_attachments(
                        attachments,
                        server_api.clone(),
                        attachments_dir.clone(),
                    )
                    .await
                }
                Err(error) => Err(error),
            }
        };
        let handoff = async {
            if !FeatureFlag::OzHandoff.is_enabled() {
                return Ok(None);
            }
            driver::attachments::fetch_and_download_handoff_snapshot_attachments(
                ai_client.clone(),
                server_api.http_client(),
                task_id,
                attachments_dir.clone(),
            )
            .await
        };
        let (regular_result, handoff_result) = futures::join!(regular, handoff);
        let mut attachments_dir = match regular_result {
            Ok(dir) => dir,
            Err(error) => {
                log::warn!("Failed to fetch and download attachments: {error:#}");
                None
            }
        };
        match handoff_result {
            Ok(Some(dir)) => {
                attachments_dir.get_or_insert(dir);
            }
            Ok(None) => {}
            Err(error) => log::warn!("Failed to fetch handoff snapshot attachments: {error:#}"),
        }
        driver_options.secrets = secrets;
        if let AgentRunPrompt::ServerSide {
            attachments_dir: ref mut dir,
            ..
        } = task.prompt
        {
            *dir = attachments_dir;
        }
        Ok(())
    }

    /// For task-only launches, fetch secrets, task metadata, and task attachments (images and
    /// files) from the server and update the driver options.
    ///
    /// Returns the task's `conversation_id` when the server has linked the task to an existing
    /// AI conversation (e.g. a `run-cloud --conversation` spawn). The caller uses this to drive
    /// transcript rehydration without a separate `--conversation` CLI arg.
    async fn fetch_secrets_and_attachments(
        foreground: &ModelSpawner<Self>,
        ai_client: &Arc<dyn AIClient>,
        task_id_str: String,
        driver_options: &mut AgentDriverOptions,
        task: &mut Task,
    ) -> Result<(Option<String>, Vec<SourceRepo>), AgentDriverError> {
        let (task_secrets, server_api) = foreground
            .spawn({
                let task_id_str = task_id_str.clone();
                move |_, ctx| {
                    let task_secrets = ManagedSecretManager::handle(ctx)
                        .as_ref(ctx)
                        .get_task_secrets(task_id_str);
                    let server_api = ServerApiProvider::handle(ctx).as_ref(ctx).get();
                    (task_secrets, server_api)
                }
            })
            .await?;

        let parsed_task_id = match task_id_str.parse().context("Failed to parse task ID") {
            Ok(id) => Some(id),
            Err(e) => {
                report_error!(e);
                None
            }
        };
        // Set the task ID on the ServerApi before any task-scoped server calls below, so failures
        // during setup can still be reported with cloud-agent context.
        Self::set_ambient_agent_task_id(foreground, parsed_task_id).await?;

        // Fetch secrets, task metadata, regular attachments, and handoff snapshot
        // attachments in parallel. The handoff snapshot fetch is independent of the
        // other three calls and only shares the download dir (a cloned PathBuf).
        let attachments_download_dir = attachments_download_dir(&driver_options.working_dir);
        let task_ai_client = ai_client.clone();
        let task_metadata = async {
            match parsed_task_id {
                Some(task_id) => task_ai_client
                    .get_ambient_agent_task(&task_id)
                    .await
                    .map(Some),
                None => Ok(None),
            }
        };

        // Handoff snapshot attachments for follow-up executions are written to
        // {attachments_dir}/handoff/{filename} so the server-side rehydration prompt
        // references resolve to real files.
        let handoff_snapshot_ai_client = ai_client.clone();
        let handoff_snapshot_server_api = server_api.clone();
        let handoff_snapshot_download_dir = attachments_download_dir.clone();
        let handoff_snapshot = async move {
            if !FeatureFlag::OzHandoff.is_enabled() {
                return Ok(None);
            }
            let Some(task_id_parsed) = parsed_task_id else {
                return Ok(None);
            };
            driver::attachments::fetch_and_download_handoff_snapshot_attachments(
                handoff_snapshot_ai_client,
                handoff_snapshot_server_api.http_client(),
                task_id_parsed,
                handoff_snapshot_download_dir,
            )
            .await
        };

        let (secrets_result, attachments_result, task_metadata_result, handoff_snapshot_result) = futures::join!(
            task_secrets,
            driver::attachments::fetch_and_download_attachments(
                ai_client.clone(),
                server_api.clone(),
                task_id_str.clone(),
                attachments_download_dir.clone(),
            ),
            task_metadata,
            handoff_snapshot,
        );

        // Extract attachments_dir from successful result, log errors
        let mut attachments_dir = match attachments_result {
            Ok(dir) => dir,
            Err(e) => {
                log::warn!("Failed to fetch and download attachments: {e:#}");
                None
            }
        };

        match handoff_snapshot_result {
            Ok(Some(dir)) => {
                // Ensure attachments_dir is set so it's passed to the server even when
                // there were no regular task attachments.
                attachments_dir.get_or_insert(dir);
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("Failed to fetch handoff snapshot attachments: {e:#}");
            }
        }

        let secrets = match secrets_result {
            Ok(secrets) => secrets,
            Err(err) => {
                // Ignore errors due to running in a non-isolated environment.
                // Otherwise, fail fast - we should not start the driver without secrets
                // in an environment where they should be available.
                if err
                    .downcast_ref::<IsolationPlatformError>()
                    .is_some_and(|err| {
                        matches!(err, IsolationPlatformError::NoIsolationPlatformDetected)
                    })
                {
                    Default::default()
                } else {
                    return Err(AgentDriverError::SecretsFetchFailed(err));
                }
            }
        };
        let (
            parent_run_id,
            task_conversation_id,
            task_harness,
            task_harness_model_config,
            additional_source_repos,
            task_team_scope,
            experimental,
        ) = match task_metadata_result {
            Ok(Some(task_metadata)) => {
                // The task's harness is stored on the snapshot; if absent, it's the default Oz.
                let agent_config_snapshot = task_metadata.agent_config_snapshot;
                let task_harness_config = agent_config_snapshot
                    .as_ref()
                    .and_then(|c| c.harness.as_ref());
                let task_harness = task_harness_config
                    .map(|h| h.harness_type)
                    .unwrap_or(Harness::Oz);
                let task_harness_model_config = task_harness_config.and_then(|h| h.model_config());
                let experimental = agent_config_snapshot
                    .as_ref()
                    .and_then(|config| config.experimental.clone());
                let additional_source_repos = agent_config_snapshot
                    .and_then(|config| config.additional_source_repos)
                    .unwrap_or_default();
                let task_team_scope = task_metadata
                    .scope
                    .as_ref()
                    .map(HeadlessTeamScope::from_task_scope);
                (
                    task_metadata.parent_run_id,
                    task_metadata.conversation_id,
                    Some(task_harness),
                    task_harness_model_config,
                    additional_source_repos,
                    task_team_scope,
                    experimental,
                )
            }
            Ok(None) => (None, None, None, None, Vec::new(), None, None),
            Err(err) => return Err(AgentDriverError::TaskMetadataFetchFailed(err)),
        };
        match experimental.as_ref() {
            Some(values) => warp_core::safe_info!(
                safe: ("factory_experimental_config state=read_uninterpreted key_count={}", values.len()),
                full: ("factory_experimental_config state=read_uninterpreted key_count={}", values.len())
            ),
            None => warp_core::safe_info!(
                safe: ("factory_experimental_config state=absent key_count=0"),
                full: ("factory_experimental_config state=absent key_count=0")
            ),
        }

        // Validate the requested `--harness` against the task's harness setting. This avoids the
        // extra conversation-metadata roundtrip that would otherwise be needed downstream when the
        // task is linked to an existing conversation, since task harness and conversation harness
        // always match (the task spawned the conversation).
        if let Some(task_harness) = task_harness {
            task.harness = reconcile_task_harness(
                &task_id_str,
                &mut driver_options.selected_harness,
                task_harness,
                driver_options.harness_transport,
            )?;
        }

        driver_options.task_id = parsed_task_id;
        driver_options.parent_run_id = parent_run_id;
        driver_options.experimental = experimental;
        driver_options.secrets = secrets;
        // The server-reported task scope is authoritative for the headless window this run
        // creates; it supersedes whatever scope was resolved from CLI args before the task was
        // fetched. Older servers that don't send `scope` fall back to that earlier resolution.
        if let Some(task_team_scope) = task_team_scope {
            driver_options.team_scope = Some(task_team_scope);
        }
        // CLI flags continue to take precedence so users can still override per-invocation.
        if driver_options.third_party_harness_model_config.is_none() {
            driver_options.third_party_harness_model_config = task_harness_model_config;
        }

        // Update the task prompt to include the downloaded attachments dir
        if let AgentRunPrompt::ServerSide {
            attachments_dir: ref mut dir,
            ..
        } = task.prompt
        {
            *dir = attachments_dir;
        }

        Ok((task_conversation_id, additional_source_repos))
    }

    /// If we are starting this agent run from an existing conversation, load the conversation
    /// data from the server and return the harness-specific [`ResumeOptions`] payload that the
    /// caller plugs onto [`AgentDriverOptions::resume`].
    ///
    /// `harness` is the resolved harness from the task config (already validated against the
    /// conversation's metadata up-front by [`common::fetch_and_validate_conversation_harness`]).
    ///
    /// For the Oz harness, and for third-party harnesses that drive a native conversation,
    /// fetches the full conversation and returns a [`driver::ResumeOptions::Native`] restoration.
    /// Otherwise, delegates to [`ThirdPartyHarness::fetch_resume_payload`] and wraps the returned
    /// payload (if any) in [`driver::ResumeOptions::ThirdParty`]; each harness owns its server
    /// call and error mapping. Returns `None` if a third-party harness has no resume payload to
    /// surface.
    #[tracing::instrument(skip_all, err, fields(tags.cloud_agent = true, conversation_id = conversation_id))]
    async fn load_conversation_information(
        foreground: &ModelSpawner<Self>,
        conversation_id: String,
        harness: &HarnessKind,
    ) -> Result<Option<driver::ResumeOptions>, AgentDriverError> {
        match harness {
            HarnessKind::Oz => {
                let (conversation_data, metadata) =
                    Self::fetch_server_conversation(foreground, &conversation_id)
                        .await
                        .map_err(conversation_load_failed)?;
                Self::native_resume_options(&conversation_data, metadata).map(Some)
            }
            HarnessKind::ThirdParty(h) => {
                if !h.drives_cli_agent_session() {
                    // A run that ended before its first save left no history, and the server
                    // does not list a conversation until it has some; the run then continues the
                    // same server conversation from an empty native one.
                    match Self::fetch_server_conversation(foreground, &conversation_id).await {
                        Ok((conversation_data, metadata))
                            if !conversation_data.tasks.is_empty() =>
                        {
                            return Self::native_resume_options(&conversation_data, metadata)
                                .map(Some);
                        }
                        Ok(_) => {}
                        Err(error) if error.downcast_ref::<ConversationNotFound>().is_some() => {}
                        Err(error) => return Err(conversation_load_failed(error)),
                    }
                    log::info!(
                        "Conversation {conversation_id} has no stored native history; resuming it empty"
                    );
                }
                let harness_support_client = foreground
                    .spawn(|_, ctx| ServerApiProvider::as_ref(ctx).get_harness_support_client())
                    .await?;
                let resume_conversation_id = ServerConversationToken::new(conversation_id.clone());
                Ok(
                    h.fetch_resume_payload(&resume_conversation_id, harness_support_client)
                        .await?
                        .map(|payload| driver::ResumeOptions::ThirdParty(Box::new(payload))),
                )
            }
            HarnessKind::Unsupported(harness) => Err(AgentDriverError::HarnessSetupFailed {
                harness: harness.to_string(),
                reason: format!(
                    "The {harness} harness is only supported for local child agent launches."
                ),
            }),
        }
    }

    async fn fetch_server_conversation(
        foreground: &ModelSpawner<Self>,
        conversation_id: &str,
    ) -> anyhow::Result<(ConversationData, ServerAIConversationMetadata)> {
        let server_api = foreground
            .spawn(|_, ctx| {
                ServerApiProvider::handle(ctx)
                    .as_ref(ctx)
                    .get_ai_client()
                    .clone()
            })
            .await?;
        server_api
            .get_ai_conversation(ServerConversationToken::new(conversation_id.to_owned()))
            .await
    }

    /// Restores the conversation into the run's terminal so the run continues it.
    fn native_resume_options(
        conversation_data: &ConversationData,
        metadata: ServerAIConversationMetadata,
    ) -> Result<driver::ResumeOptions, AgentDriverError> {
        let conversation = convert_conversation_data_to_ai_conversation(
            AIConversationId::default(),
            conversation_data,
            metadata,
            RestorationMode::Continue,
        )
        .ok_or_else(|| {
            AgentDriverError::ConversationLoadFailed(
                "Failed to convert conversation data to AIConversation".into(),
            )
        })?;
        Ok(driver::ResumeOptions::Native(Box::new(
            ConversationRestorationInNewPaneType::Historical {
                conversation,
                should_use_live_appearance: false,
                ambient_agent_task_id: None,
            },
        )))
    }

    /// Resolve the environment and store into `driver_options`.
    #[tracing::instrument(skip_all, err, fields(tags.cloud_agent = true, ?environment_id))]
    async fn resolve_environment(
        foreground: &ModelSpawner<Self>,
        environment_id: Option<String>,
        driver_options: &mut AgentDriverOptions,
    ) -> Result<Option<AmbientAgentEnvironment>, AgentDriverError> {
        let Some(environment_id) = environment_id else {
            return Ok(None);
        };

        let environment = foreground
            .spawn(move |_, ctx| -> Result<_, AgentDriverError> {
                let server_id = ServerId::try_from(environment_id.as_str()).map_err(|_| {
                    report_error!(
                        "Invalid environment ID",
                        extra: { "environment_id" => %environment_id }
                    );
                    AgentDriver::log_valid_environments(ctx);
                    AgentDriverError::EnvironmentNotFound(environment_id.clone())
                })?;
                let sync_id = SyncId::ServerId(server_id);

                CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx)
                    .ok_or_else(|| {
                        report_error!(
                            "Environment not found with ID",
                            extra: { "environment_id" => %environment_id }
                        );
                        AgentDriver::log_valid_environments(ctx);
                        AgentDriverError::EnvironmentNotFound(environment_id)
                    })
                    .map(|env| env.model().string_model.clone())
            })
            .await??;

        if FeatureFlag::OzIdentityFederation.is_enabled() {
            let run_id = driver_options
                .task_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "local".to_string());
            driver_options.cloud_providers =
                driver::cloud_provider::load_providers(&environment.providers, &run_id)
                    .map_err(AgentDriverError::CloudProviderSetupFailed)?;
        }

        Ok(Some(environment))
    }

    /// Create the AgentDriver and start running the task.
    #[tracing::instrument(skip_all, fields(tags.cloud_agent = true))]
    fn create_and_run_driver(
        ctx: &mut AppContext,
        driver_options: driver::AgentDriverOptions,
        output_format: OutputFormat,
        share_requests: Option<Vec<ShareRequest>>,
        task: driver::Task,
    ) {
        maybe_warn_team_api_key(ctx);

        // Initializing the driver will fail if not logged in. Since we check that above, panic here - it's difficult to
        // fallibly instantiate a UI framework model.
        let driver = ctx.add_singleton_model(|ctx| {
            AgentDriver::new(driver_options, ctx).expect("Could not initialize driver")
        });

        driver.update(ctx, |driver, ctx| {
            driver.set_output_format(output_format);
            if let Some(share_requests) = share_requests {
                driver.add_share_requests(share_requests, ctx);
            }
            let span =
                tracing::info_span!("AgentDriver::run", tags.cloud_agent = true, ?task.model, ?task.harness);
            let agent_future = span
                .in_scope(|| driver.run(task, ctx))
                .instrument(span);

            ctx.spawn(agent_future, |_, result, ctx| match result {
                Ok(()) => {
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    report_fatal_error(err.into(), ctx);
                }
            });
        });
    }
}

fn conversation_load_failed(error: anyhow::Error) -> AgentDriverError {
    AgentDriverError::ConversationLoadFailed(format!("{error}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CommandAuthentication {
    PendingApiKey(String),
    RefreshUser,
}

fn command_authentication(
    pending_api_key: Option<String>,
    is_logged_in: bool,
) -> Option<CommandAuthentication> {
    match pending_api_key {
        Some(api_key) => Some(CommandAuthentication::PendingApiKey(api_key)),
        None if is_logged_in => Some(CommandAuthentication::RefreshUser),
        None => None,
    }
}

/// Returns `true` if the given CLI command requires authentication.
fn command_requires_auth(command: &CliCommand) -> bool {
    match command {
        CliCommand::Agent(agent_cmd) => match agent_cmd {
            AgentCommand::Run { .. } => true,
            AgentCommand::RunCloud { .. } => true,
            AgentCommand::Profile(sub) => match sub {
                AgentProfileCommand::List => true,
            },
            AgentCommand::List(_) => true,
            AgentCommand::Get(_) => true,
            AgentCommand::Create(_) => true,
            AgentCommand::Update(_) => true,
            AgentCommand::Delete(_) => true,
            AgentCommand::Skills(_) => true,
        },
        CliCommand::Environment(environment_cmd) => match environment_cmd {
            EnvironmentCommand::List { .. } => true,
            EnvironmentCommand::Create { .. } => true,
            EnvironmentCommand::Delete { .. } => true,
            EnvironmentCommand::Update { .. } => true,
            EnvironmentCommand::Get { .. } => true,
            EnvironmentCommand::Image(ImageCommand::List) => true,
        },
        CliCommand::MCP(mcp_cmd) => match mcp_cmd {
            MCPCommand::List => true,
        },
        CliCommand::Run(task_cmd) => match task_cmd {
            TaskCommand::List { .. } => true,
            TaskCommand::Get { .. } => true,
            TaskCommand::Conversation { .. } => true,
            TaskCommand::Message { .. } => true,
        },
        CliCommand::Model(model_cmd) => match model_cmd {
            ModelCommand::List(_) => true,
        },
        CliCommand::MemoryStore(_) => true,
        CliCommand::EnvironmentCheckout(_) => false,
        CliCommand::Memory(_) => true,
        CliCommand::Login => false,
        CliCommand::Logout => false,
        CliCommand::Whoami => true,
        CliCommand::Provider(_) => true,
        CliCommand::Integration(_) => true,
        CliCommand::Schedule(_) => true,
        CliCommand::Secret(_) => true,
        CliCommand::Federate(_) => true,
        CliCommand::HarnessSupport(_) => true,
        CliCommand::Artifact(_) => true,
        CliCommand::ApiKey(_) => true,
        CliCommand::Runner(_) => true,
    }
}

/// Launch a CLI command, checking authentication first if needed.
///
/// If auth is not required, dispatches the command immediately.
/// If auth is required, validates an explicit API key or refreshes persisted
/// credentials before launching the command.
fn launch_command(
    ctx: &mut AppContext,
    command: CliCommand,
    global_options: GlobalOptions,
) -> anyhow::Result<()> {
    let parent_span = tracing::Span::current();
    let requires_auth = command_requires_auth(&command);

    if !requires_auth {
        return dispatch_command(ctx, command, global_options);
    }

    let cli_name = warp_cli::binary_name().unwrap_or_else(|| "warp".to_string());

    let auth_state = AuthStateProvider::handle(ctx).as_ref(ctx).get();
    let Some(authentication) =
        command_authentication(global_options.api_key.clone(), auth_state.is_logged_in())
    else {
        return Err(anyhow::anyhow!(
            "You are not logged in - please log in with `{cli_name} login` to continue."
        ));
    };

    // On staging the warp-server is fronted by IAP, so establish an IAP token
    // before *any* warp-server request.
    let iap = IapManager::handle(ctx);
    if !iap.as_ref(ctx).is_enabled() || iap.as_ref(ctx).has_valid_token() {
        authenticate_and_dispatch(ctx, command, global_options, authentication, parent_span);
        return Ok(());
    }

    let mut handled = false;
    let parent_span_for_iap = parent_span.clone();
    ctx.subscribe_to_model(&iap, move |_, event, ctx| {
        let _guard = parent_span_for_iap.enter();
        if handled {
            return;
        }
        match event {
            IapManagerEvent::StateChanged
                if IapManager::handle(ctx).as_ref(ctx).has_valid_token() =>
            {
                handled = true;
                authenticate_and_dispatch(
                    ctx,
                    command.clone(),
                    global_options.clone(),
                    authentication.clone(),
                    parent_span.clone(),
                );
            }
            IapManagerEvent::AccessUnavailable => {
                handled = true;
                report_fatal_error(
                    anyhow::anyhow!("Timed out establishing IAP access to warp-server."),
                    ctx,
                );
            }
            _ => {}
        }
    });

    iap.update(ctx, |manager, ctx| manager.ensure_access(ctx));

    Ok(())
}

/// Subscribes to auth events, authenticates, and dispatches the command once
/// auth completes. Assumes IAP access (if applicable) is already established.
fn authenticate_and_dispatch(
    ctx: &mut AppContext,
    command: CliCommand,
    global_options: GlobalOptions,
    authentication: CommandAuthentication,
    parent_span: tracing::Span,
) {
    let cli_name = warp_cli::binary_name().unwrap_or_else(|| "warp".to_string());

    // Subscribe to auth events and wait for validation before running the command.
    let mut dispatched = false;
    ctx.subscribe_to_model(&AuthManager::handle(ctx), move |_, event, ctx| {
        let _guard = parent_span.enter();
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
                    "Your API key is invalid. Please provide a valid key via '--api-key' or the WARP_API_KEY environment variable.".to_string()
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

    // Trigger authentication - the subscription above will handle the result.
    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| match authentication {
        CommandAuthentication::PendingApiKey(api_key) => {
            auth_manager.authenticate_api_key(api_key, ctx);
        }
        CommandAuthentication::RefreshUser => auth_manager.refresh_user(ctx),
    });
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

fn resolve_orchestration_harness_label() -> &'static str {
    let Ok(raw) = std::env::var(OZ_HARNESS_ENV) else {
        return "unknown";
    };
    match Harness::parse_orchestration_harness(&raw) {
        Some(Harness::Oz) => "oz",
        Some(Harness::Claude) => "claude",
        Some(Harness::OpenCode) => "opencode",
        Some(Harness::Gemini) => "gemini",
        Some(Harness::Codex) => "codex",
        Some(Harness::Unknown) | None => "unknown",
    }
}

/// Map each CLI command into a telemetry event to emit when it's executed.
fn command_to_telemetry_event(command: &CliCommand) -> CliTelemetryEvent {
    match command {
        CliCommand::Agent(AgentCommand::Run(args)) => CliTelemetryEvent::AgentRun {
            gui: args.gui,
            requested_mcp_servers: args.mcp_specs.len() + args.mcp_servers.len(),
            has_environment: args.environment.is_some(),
            task_id: args.task_id.clone(),
            harness: args.harness.to_string(),
        },
        CliCommand::Agent(AgentCommand::RunCloud(_)) => CliTelemetryEvent::AgentRunAmbient,
        CliCommand::Agent(AgentCommand::Profile(sub)) => match sub {
            AgentProfileCommand::List => CliTelemetryEvent::AgentProfileList,
        },
        CliCommand::Agent(AgentCommand::List(_)) => CliTelemetryEvent::AgentList,
        CliCommand::Agent(AgentCommand::Get(_)) => CliTelemetryEvent::AgentGet,
        CliCommand::Agent(AgentCommand::Create(_)) => CliTelemetryEvent::AgentCreate,
        CliCommand::Agent(AgentCommand::Update(_)) => CliTelemetryEvent::AgentUpdate,
        CliCommand::Agent(AgentCommand::Delete(_)) => CliTelemetryEvent::AgentDelete,
        CliCommand::Agent(AgentCommand::Skills(_)) => CliTelemetryEvent::AgentSkills,
        CliCommand::Environment(EnvironmentCommand::List { .. }) => {
            CliTelemetryEvent::EnvironmentList
        }
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
        CliCommand::EnvironmentCheckout(_) => CliTelemetryEvent::EnvironmentCheckout,
        CliCommand::Run(TaskCommand::List(_)) => CliTelemetryEvent::TaskList,
        CliCommand::Run(TaskCommand::Get(args)) => {
            if args.conversation {
                CliTelemetryEvent::RunConversationGet
            } else {
                CliTelemetryEvent::TaskGet
            }
        }
        CliCommand::Run(TaskCommand::Conversation(_)) => CliTelemetryEvent::ConversationGet,
        CliCommand::Run(TaskCommand::Message(message_cmd)) => match message_cmd {
            MessageCommand::Watch(_) => CliTelemetryEvent::RunMessageWatch {
                harness: resolve_orchestration_harness_label(),
            },
            MessageCommand::Send(_) => CliTelemetryEvent::RunMessageSend {
                harness: resolve_orchestration_harness_label(),
            },
            MessageCommand::List(_) => CliTelemetryEvent::RunMessageList {
                harness: resolve_orchestration_harness_label(),
            },
            MessageCommand::Read(_) => CliTelemetryEvent::RunMessageRead {
                harness: resolve_orchestration_harness_label(),
            },
            MessageCommand::MarkDelivered(_) => CliTelemetryEvent::RunMessageMarkDelivered {
                harness: resolve_orchestration_harness_label(),
            },
        },
        CliCommand::Model(ModelCommand::List(_)) => CliTelemetryEvent::ModelList,
        CliCommand::MemoryStore(memory_store_cmd) => match memory_store_cmd {
            MemoryStoreCommand::List(_) => CliTelemetryEvent::MemoryStoreList,
            MemoryStoreCommand::Get(_) => CliTelemetryEvent::MemoryStoreGetStore,
            MemoryStoreCommand::Update(_) => CliTelemetryEvent::MemoryStoreUpdateStore,
            MemoryStoreCommand::ListStoreAgents(_) => CliTelemetryEvent::MemoryStoreListStoreAgents,
        },
        CliCommand::Memory(memory_cmd) => match memory_cmd {
            MemoryCommand::List(_) => CliTelemetryEvent::MemoryStoreListMemories,
            MemoryCommand::Create(_) => CliTelemetryEvent::MemoryStoreCreateMemory,
            MemoryCommand::Update(_) => CliTelemetryEvent::MemoryStoreUpdateMemory,
            MemoryCommand::Delete(_) => CliTelemetryEvent::MemoryStoreDeleteMemory,
            MemoryCommand::Versions(_) => CliTelemetryEvent::MemoryStoreListVersions,
        },
        CliCommand::Login => CliTelemetryEvent::Login,
        CliCommand::Logout => CliTelemetryEvent::Logout,
        CliCommand::Whoami => CliTelemetryEvent::Whoami,
        CliCommand::Provider(ProviderCommand::Setup(_)) => CliTelemetryEvent::ProviderSetup,
        CliCommand::Provider(ProviderCommand::List) => CliTelemetryEvent::ProviderList,
        CliCommand::Integration(integration_cmd) => match integration_cmd {
            IntegrationCommand::Create(_) => CliTelemetryEvent::IntegrationCreate,
            IntegrationCommand::Update(_) => CliTelemetryEvent::IntegrationUpdate,
            IntegrationCommand::List => CliTelemetryEvent::IntegrationList,
        },
        CliCommand::Schedule(c) => match c.subcommand() {
            None | Some(ScheduleSubcommand::Create(_)) => CliTelemetryEvent::ScheduleCreate,
            Some(ScheduleSubcommand::List) => CliTelemetryEvent::ScheduleList,
            Some(ScheduleSubcommand::Get(_)) => CliTelemetryEvent::ScheduleGet,
            Some(ScheduleSubcommand::Pause(_)) => CliTelemetryEvent::SchedulePause,
            Some(ScheduleSubcommand::Unpause(_)) => CliTelemetryEvent::ScheduleUnpause,
            Some(ScheduleSubcommand::Update(_)) => CliTelemetryEvent::ScheduleUpdate,
            Some(ScheduleSubcommand::Delete(_)) => CliTelemetryEvent::ScheduleDelete,
        },
        CliCommand::Secret(secret_cmd) => match secret_cmd {
            SecretCommand::Create(_) => CliTelemetryEvent::SecretCreate,
            SecretCommand::Delete(_) => CliTelemetryEvent::SecretDelete,
            SecretCommand::Update(_) => CliTelemetryEvent::SecretUpdate,
            SecretCommand::List(_) => CliTelemetryEvent::SecretList,
        },
        CliCommand::Federate(federate_cmd) => match federate_cmd {
            FederateCommand::IssueToken(_) => CliTelemetryEvent::FederateIssueToken,
            FederateCommand::IssueGcpToken(_) => CliTelemetryEvent::FederateIssueGcpToken,
        },
        CliCommand::HarnessSupport(args) => match &args.command {
            HarnessSupportCommand::Ping => CliTelemetryEvent::HarnessSupportPing,
            HarnessSupportCommand::ReportArtifact(report_args) => match &report_args.command {
                ReportArtifactCommand::PullRequest(_) => {
                    CliTelemetryEvent::HarnessSupportReportArtifact {
                        artifact_type: "pull_request",
                    }
                }
            },
            HarnessSupportCommand::ReportExternalReference(_) => {
                CliTelemetryEvent::HarnessSupportReportArtifact {
                    artifact_type: "external_reference",
                }
            }
            HarnessSupportCommand::NotifyUser(_) => CliTelemetryEvent::HarnessSupportNotifyUser,
            HarnessSupportCommand::FinishTask(finish_args) => {
                CliTelemetryEvent::HarnessSupportFinishTask {
                    success: finish_args.status == TaskStatus::Success,
                }
            }
            HarnessSupportCommand::ReportShutdown(_) => {
                CliTelemetryEvent::HarnessSupportReportShutdown
            }
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
        CliCommand::Runner(runner_cmd) => match runner_cmd {
            RunnerCommand::List(_) => CliTelemetryEvent::RunnerList,
            RunnerCommand::Create(_) => CliTelemetryEvent::RunnerCreate,
            RunnerCommand::Update(_) => CliTelemetryEvent::RunnerUpdate,
            RunnerCommand::Delete(_) => CliTelemetryEvent::RunnerDelete,
        },
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
