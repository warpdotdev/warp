//! Agent SDK entry points for invoking Agent-related functionality from the app.
//! For now this provides a simple runner that echoes the received command.

use std::fmt::Write;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use ai::api_keys::{ApiKeyManager, AwsCredentialsRefreshStrategy};
use anyhow::Context;
pub(crate) use driver::harness::{task_env_vars, validate_cli_installed, ClaudeHarness};
pub use driver::AgentDriver;
use driver::AgentDriverError;
use telemetry::CliTelemetryEvent;
use warp_cli::agent::{
    AgentCommand, AgentProfileCommand, Harness, OutputFormat, Prompt, RunAgentArgs,
};
use warp_cli::api_key::ApiKeyCommand;
use warp_cli::artifact::ArtifactCommand;
use warp_cli::environment::{EnvironmentCommand, ImageCommand};
use warp_cli::federate::FederateCommand;
use warp_cli::harness_support::{HarnessSupportCommand, ReportArtifactCommand, TaskStatus};
use warp_cli::integration::IntegrationCommand;
use warp_cli::mcp::MCPCommand;
use warp_cli::model::ModelCommand;
use warp_cli::provider::ProviderCommand;
use warp_cli::schedule::ScheduleSubcommand;
use warp_cli::secret::SecretCommand;
use warp_cli::share::ShareRequest;
use warp_cli::task::{MessageCommand, TaskCommand};
use warp_cli::{CliCommand, GlobalOptions, OZ_HARNESS_ENV};
use warp_core::features::FeatureFlag;
use warp_graphql::object_permissions::OwnerType;
use warp_isolation_platform::IsolationPlatformError;
#[cfg(not(target_family = "wasm"))]
use warp_logging::log_file_path;
use warp_managed_secrets::ManagedSecretManager;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelSpawner, SingletonEntity};

use crate::ai::agent::api::convert_conversation::{
    convert_conversation_data_to_ai_conversation, RestorationMode,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_sdk::driver::harness::{harness_kind, HarnessKind};
use crate::ai::agent_sdk::driver::{AgentDriverOptions, AgentRunPrompt, Task};
use crate::ai::agent_sdk::mcp_config::build_mcp_servers_from_specs;
use crate::ai::agent_sdk::setup_observability::{
    SetupClientEventReporter, SetupStep, SetupTimelineEvent,
};
use crate::ai::ambient_agents::task::HarnessConfig;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::attachment_utils::attachments_download_dir;
#[cfg(not(target_family = "wasm"))]
use crate::ai::aws_credentials::refresh_aws_credentials;
use crate::ai::cloud_environments::CloudAmbientAgentEnvironment;
use crate::ai::llms::LLMId;
use crate::ai::skills::{
    clone_repo_for_skill, resolve_skill_spec, ResolveSkillError, ResolvedSkill,
};
use crate::auth::auth_manager::{AuthManager, AuthManagerEvent};
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::CloudObjectLookup as _;
use crate::send_telemetry_sync_from_app_ctx;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ai::{AIClient, AgentConfigSnapshot};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::view::ConversationRestorationInNewPaneType;
use crate::workflows::workflow::Workflow;

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
mod federate;
mod harness_support;
#[cfg(not(target_family = "wasm"))]
mod integration;
#[cfg(not(target_family = "wasm"))]
mod integration_output;
mod mcp;
mod mcp_config;
mod model;
mod oauth_flow;
pub mod output;
mod profiles;
mod provider;
pub(crate) mod retry;
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

/// Run a Warp CLI command.
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
        CliCommand::MCP(mcp_cmd) => mcp::run(ctx, global_options, mcp_cmd),
        CliCommand::Run(task_cmd) => run_task(ctx, global_options, task_cmd),
        CliCommand::Model(model_cmd) => model::run(ctx, global_options, model_cmd),
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
            if args.claude_auth_secret.is_some() && args.harness != Harness::Claude {
                return Err(anyhow::anyhow!(
                    "--claude-auth-secret is only valid with --harness claude."
                ));
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
        // CLI name > skill name > file name
        name: args.name.clone().or(skill_name).or(file_merged.name),
        environment_id: args.environment.clone().or(file_merged.environment_id),
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
    };

    let runtime_mcp_specs = match merged_config.mcp_servers.as_ref() {
        Some(mcp_servers) => config_file::mcp_specs_from_mcp_servers(mcp_servers)?,
        None => Vec::new(),
    };

    let model_override: Option<LLMId> = merged_config
        .model_id
        .as_deref()
        .filter(|_| args.harness == Harness::Oz)
        .map(|model_id| common::validate_agent_mode_base_model_id(model_id, ctx))
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
        harness: harness_kind(args.harness)?,
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
        name: args.name.clone().or(skill_name),
        environment_id: environment.clone(),
        model_id: model_id_string,
        base_prompt: None,
        mcp_servers: cli_mcp_servers,
        profile_id: profile.clone(),
        worker_host: None,
        skill_spec: None,
        computer_use_enabled: args.computer_use.computer_use_override(),
        harness: harness_override,
        harness_auth_secrets: None,
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
        harness: harness_kind(args.harness)?,
    };

    Ok((config, task))
}

fn reconcile_task_harness(
    task_id: &str,
    selected_harness: &mut Harness,
    task_harness: Harness,
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

    harness_kind(*selected_harness)
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
        TaskCommand::Message(message_cmd) => {
            if !FeatureFlag::OrchestrationV2.is_enabled() {
                return Err(anyhow::anyhow!(
                    "The 'message' subcommand is not available in this build"
                ));
            }
            ambient::run_message(ctx, global_options, message_cmd)
        }
    }
}

/// Singleton model that provides a ModelContext for spawning async operations
/// when starting the agent driver. This is needed because conversation fetching
/// requires spawning an async task, which requires a ModelContext.
struct AgentDriverRunner;

impl warpui::Entity for AgentDriverRunner {
    type Event = ();
}

impl warpui::SingletonEntity for AgentDriverRunner {}

impl AgentDriverRunner {
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
        Self::set_ambient_agent_task_id(&foreground, task_id).await?;
        let background = foreground.spawn(|_, ctx| ctx.background_executor()).await?;
        let setup_events = match task_id {
            Some(task_id) => SetupClientEventReporter::new(task_id, server_api.clone(), background),
            None => SetupClientEventReporter::noop(server_api.clone(), background),
        };
        setup_events
            .post_timeline_event(SetupTimelineEvent::WorkerContainerReady)
            .await;

        // Ensure we've synced team state before starting the driver.
        setup_events
            .record_result(
                SetupStep::TeamMetadataRefresh,
                Self::refresh_team_metadata(&foreground),
            )
            .await?;

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

        // Set up and run the driver, reporting any errors back to the server.
        let result: Result<(), AgentDriverError> = async {
            // Pull relevant variables out of args before moving it into the closure.
            let share_requests = args.share.share.clone();
            let bedrock_inference_role = args.bedrock_inference_role.clone();
            let bedrock_role_region = args.bedrock_role_region.clone();
            let has_task_id = args.task_id.is_some();
            let args_harness = args.harness;
            // `--conversation` path (user-invoked local resume): validate before any task side
            // effects so mismatches fail fast. The `--task-id` path derives its conversation id
            // from the server-side task metadata inside `build_driver_options_and_task`. Both
            // can currently be passed together (the worker server-side appends `--conversation`
            // alongside `--task-id` for Slack/Linear followups); when both are set, the explicit
            // `--conversation` value wins via the merge below.
            if !has_task_id {
                if let Some(conversation_id) = args.conversation.as_deref() {
                    common::fetch_and_validate_conversation_harness(
                        server_api.clone(),
                        conversation_id,
                        args_harness,
                    )
                    .await?;
                }
            }
            let resume_conversation_id = args.conversation.clone();

            // Build driver options and task, handling task creation or existing task setup.
            // For the `--task-id` path, `task_conversation_id` is the `conversation_id` read off
            // the fetched `AmbientAgentTask` (set by the server when linking the task to an
            // existing conversation, e.g. via `run-cloud --conversation`).
            let (mut driver_options, task, task_conversation_id) =
                Self::build_driver_options_and_task(&foreground, args, &server_api, &setup_events)
                    .await?;

            // Update the effective task ID so errors are reported correctly.
            // This only matters if we created a task ID locally.
            task_id = driver_options.task_id.or(task_id);

            // The `--task-id` branch already validated `args_harness` against the task's harness
            // setting inside `build_driver_options_and_task`; the conversation that the task spawned
            // necessarily uses the same harness, so no extra conversation-metadata roundtrip is
            // needed here. Just merge the task's linked conversation id into the resume target.
            let resume_conversation_id = resume_conversation_id.or(task_conversation_id);

            let bedrock_task_id = driver_options.task_id.map(|id| id.to_string());

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
                // Set the OIDC strategy on the UI thread and kick off the refresh; the
                // returned future resolves when credentials are committed to the model.
                let refresh_future = foreground
                    .spawn(move |_, ctx| {
                        ApiKeyManager::handle(ctx).update(ctx, |manager, ctx| {
                            // From here on, refresh credentials via OIDC federation only.
                            manager.set_aws_credentials_refresh_strategy(
                                AwsCredentialsRefreshStrategy::OidcManaged {
                                    task_id: bedrock_task_id,
                                    role_arn,
                                    region: role_region,
                                },
                            );
                            refresh_aws_credentials(manager, ctx)
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

        if let Err(ref err) = result {
            if let Some(task_id) = task_id {
                driver::report_driver_error(task_id, err, &server_api).await;
            }
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
            .map_err(|_| AgentDriverError::TeamMetadataRefreshTimeout)
    }

    async fn set_ambient_agent_task_id(
        foreground: &ModelSpawner<Self>,
        task_id: Option<AmbientAgentTaskId>,
    ) -> Result<(), AgentDriverError> {
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

    /// Build the AgentDriverOptions and Task, handling task creation or existing task setup.
    ///
    /// The third tuple element is the conversation id read off the server-side task metadata
    /// on the `--task-id` branch. It's `None` when no task id was passed or when the task is
    /// not linked to a conversation; callers use it to drive `--task-id`-implied resume
    /// without requiring the caller to also pass `--conversation`.
    async fn build_driver_options_and_task(
        foreground: &ModelSpawner<Self>,
        args: RunAgentArgs,
        server_api: &Arc<dyn AIClient>,
        setup_events: &SetupClientEventReporter,
    ) -> Result<(AgentDriverOptions, Task, Option<String>), AgentDriverError> {
        // Get the working directory
        let working_dir = match args.cwd.as_ref() {
            Some(dir) => dunce::canonicalize(dir)
                .with_context(|| format!("Unable to resolve {}", dir.display())),
            None => std::env::current_dir().context("Unable to determine working directory"),
        }
        .map_err(AgentDriverError::ConfigBuildFailed)?;

        // Resolve the skill, if we have one
        let resolved_skill =
            Self::resolve_skill(foreground, &args, &working_dir, setup_events).await?;

        // Extract variables we want to use later before moving args into the closure
        let task_id_str = args.task_id.clone();
        let prompt = args.prompt_arg.to_prompt();
        let skill = args.skill.clone();

        // Build the AgentConfigSnapshot, Task, and AgentDriverOptions
        let prompt_clone = prompt.clone();
        let (merged_config, mut task, mut driver_options) = foreground
            .spawn(move |_, ctx| -> anyhow::Result<_> {
                let (merged_config, task) =
                    build_merged_config_and_task(&args, &resolved_skill, &prompt_clone, ctx)?;

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
                    parent_run_id: None,
                    should_share,
                    idle_on_complete: args.idle_on_complete.map(|d| d.into()),
                    secrets: Default::default(),
                    resume: None,
                    cloud_providers: Vec::new(),
                    environment: None,
                    selected_harness: args.harness,
                    third_party_harness_model_config,
                    snapshot_disabled: args.snapshot.no_snapshot.then_some(true),
                    snapshot_upload_timeout: args
                        .snapshot
                        .snapshot_upload_timeout
                        .map(|duration| duration.into()),
                    snapshot_script_timeout: args
                        .snapshot
                        .snapshot_script_timeout
                        .map(|duration| duration.into()),
                };

                Ok((merged_config, task, driver_options))
            })
            .await?
            .map_err(AgentDriverError::ConfigBuildFailed)?;

        let environment_id = merged_config.environment_id.clone();

        // Handle secrets/attachments fetch (existing task) or task creation (new run).
        // The existing-task branch also surfaces the task's `conversation_id` (if any) so
        // the caller can wire up resume without a separate `--conversation` arg.
        let task_conversation_id = if let Some(task_id_str) = task_id_str {
            setup_events
                .record_result(
                    SetupStep::TaskDataFetch,
                    Self::fetch_secrets_and_attachments(
                        foreground,
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
                &mut driver_options,
            )
            .await?;
            None
        };
        // Resolve environment and cloud providers.
        setup_events
            .record_result(
                SetupStep::EnvironmentResolution,
                Self::resolve_environment(foreground, environment_id, &mut driver_options),
            )
            .await?;

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
        driver_options: &mut AgentDriverOptions,
    ) -> Result<(), AgentDriverError> {
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
            .create_agent_task(prompt, environment, None, task_config)
            .await
        {
            Ok(id) => {
                log::info!("Created task: {id}");
                Some(id)
            }
            Err(e) => {
                log::error!("Failed to create task: {e}");
                // Continue without a task_id rather than failing entirely
                None
            }
        };

        // Set the task ID on the ServerApi so it's sent with all subsequent requests.
        Self::set_ambient_agent_task_id(foreground, task_id).await?;
        driver_options.task_id = task_id;

        Ok(())
    }

    /// When starting an agent run from an existing task_id, fetch secrets, task metadata,
    /// and task attachments (images and files) from the server and update the driver options.
    ///
    /// Returns the task's `conversation_id` when the server has linked the task to an existing
    /// AI conversation (e.g. a `run-cloud --conversation` spawn). The caller uses this to drive
    /// transcript rehydration without a separate `--conversation` CLI arg.
    async fn fetch_secrets_and_attachments(
        foreground: &ModelSpawner<Self>,
        task_id_str: String,
        driver_options: &mut AgentDriverOptions,
        task: &mut Task,
    ) -> Result<Option<String>, AgentDriverError> {
        let (task_secrets, ai_client, server_api) = foreground
            .spawn({
                let task_id_str = task_id_str.clone();
                move |_, ctx| {
                    let task_secrets = ManagedSecretManager::handle(ctx)
                        .as_ref(ctx)
                        .get_task_secrets(task_id_str);
                    let ai_client = ServerApiProvider::handle(ctx)
                        .as_ref(ctx)
                        .get_ai_client()
                        .clone();
                    let server_api = ServerApiProvider::handle(ctx).as_ref(ctx).get();
                    (task_secrets, ai_client, server_api)
                }
            })
            .await?;

        let parsed_task_id = match task_id_str.parse() {
            Ok(id) => Some(id),
            Err(e) => {
                log::error!("Failed to parse task ID: {e}");
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
        // {attachments_dir}/handoff/{uuid} so the server-side rehydration prompt
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

        // Fetch a fresh GitHub token from the server so the driver can configure git
        // and gh credentials without relying on environment variable injection.
        let git_creds_ai_client = ai_client.clone();
        let git_creds_task_id = task_id_str.clone();
        let git_credentials = async move {
            if !FeatureFlag::GitCredentialRefresh.is_enabled() {
                return Ok(vec![]);
            }
            let workload_token = match warp_isolation_platform::issue_workload_token(Some(
                std::time::Duration::from_secs(5 * 60),
            ))
            .await
            {
                Ok(token) => token.token,
                Err(e) => {
                    // Not in an isolated environment — no workload token available.
                    log::debug!("Skipping git credentials fetch: {e}");
                    return Ok(vec![]);
                }
            };
            git_creds_ai_client
                .get_task_git_credentials(git_creds_task_id, workload_token)
                .await
        };

        let (
            secrets_result,
            attachments_result,
            task_metadata_result,
            handoff_snapshot_result,
            git_credentials_result,
        ) = futures::join!(
            task_secrets,
            driver::attachments::fetch_and_download_attachments(
                ai_client.clone(),
                server_api.clone(),
                task_id_str.clone(),
                attachments_download_dir.clone(),
            ),
            task_metadata,
            handoff_snapshot,
            git_credentials,
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

        if FeatureFlag::GitCredentialRefresh.is_enabled() {
            match git_credentials_result {
                Ok(credentials) if !credentials.is_empty() => {
                    driver::git_credentials::setup_git_config(&credentials);
                    driver::git_credentials::configure_git_identity(&credentials);
                    if let Err(e) = driver::git_credentials::write_git_credentials(&credentials) {
                        log::warn!("Failed to write git credentials: {e:#}");
                    } else {
                        log::info!("Git credentials configured from taskGitCredentials");
                    }
                }
                Ok(_) => {
                    log::debug!("No git credentials returned; skipping credential file setup");
                }
                Err(e) => {
                    log::warn!("Failed to fetch git credentials: {e:#}");
                }
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
        let (parent_run_id, task_conversation_id, task_harness, task_harness_model_config) =
            match task_metadata_result {
                Ok(Some(task_metadata)) => {
                    // The task's harness is stored on the snapshot; if absent, it's the default Oz.
                    let task_harness_config = task_metadata
                        .agent_config_snapshot
                        .as_ref()
                        .and_then(|c| c.harness.as_ref());
                    let task_harness = task_harness_config
                        .map(|h| h.harness_type)
                        .unwrap_or(Harness::Oz);
                    let task_harness_model_config =
                        task_harness_config.and_then(|h| h.model_config());
                    (
                        task_metadata.parent_run_id,
                        task_metadata.conversation_id,
                        Some(task_harness),
                        task_harness_model_config,
                    )
                }
                Ok(None) => (None, None, None, None),
                Err(err) => {
                    log::warn!("Failed to fetch task metadata: {err:#}");
                    (None, None, None, None)
                }
            };

        // Validate the requested `--harness` against the task's harness setting. This avoids the
        // extra conversation-metadata roundtrip that would otherwise be needed downstream when the
        // task is linked to an existing conversation, since task harness and conversation harness
        // always match (the task spawned the conversation).
        if let Some(task_harness) = task_harness {
            task.harness = reconcile_task_harness(
                &task_id_str,
                &mut driver_options.selected_harness,
                task_harness,
            )?;
        }

        driver_options.task_id = parsed_task_id;
        driver_options.parent_run_id = parent_run_id;
        driver_options.secrets = secrets;
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

        Ok(task_conversation_id)
    }

    /// If we are starting this agent run from an existing conversation, load the conversation
    /// data from the server and return the harness-specific [`ResumeOptions`] payload that the
    /// caller plugs onto [`AgentDriverOptions::resume`].
    ///
    /// `harness` is the resolved harness from the task config (already validated against the
    /// conversation's metadata up-front by [`common::fetch_and_validate_conversation_harness`]).
    ///
    /// For the Oz harness, fetches the full conversation and returns a [`driver::ResumeOptions::Oz`].
    /// For third-party harnesses, delegates to [`ThirdPartyHarness::fetch_resume_payload`] and
    /// wraps the returned payload (if any) in [`driver::ResumeOptions::ThirdParty`]; each harness
    /// owns its server call and error mapping. Returns `None` if a third-party harness has no
    /// resume payload to surface.
    async fn load_conversation_information(
        foreground: &ModelSpawner<Self>,
        conversation_id: String,
        harness: &HarnessKind,
    ) -> Result<Option<driver::ResumeOptions>, AgentDriverError> {
        match harness {
            HarnessKind::Oz => {
                let server_api = foreground
                    .spawn(|_, ctx| {
                        ServerApiProvider::handle(ctx)
                            .as_ref(ctx)
                            .get_ai_client()
                            .clone()
                    })
                    .await?;
                let token = ServerConversationToken::new(conversation_id.clone());
                let (conversation_data, metadata) = server_api
                    .get_ai_conversation(token)
                    .await
                    .map_err(|err| AgentDriverError::ConversationLoadFailed(format!("{err}")))?;
                let conversation = convert_conversation_data_to_ai_conversation(
                    AIConversationId::default(),
                    &conversation_data,
                    metadata,
                    RestorationMode::Continue,
                )
                .ok_or_else(|| {
                    AgentDriverError::ConversationLoadFailed(
                        "Failed to convert conversation data to AIConversation".into(),
                    )
                })?;
                Ok(Some(driver::ResumeOptions::Oz(Box::new(
                    ConversationRestorationInNewPaneType::Historical {
                        conversation,
                        should_use_live_appearance: false,
                        ambient_agent_task_id: None,
                    },
                ))))
            }
            HarnessKind::ThirdParty(h) => {
                let harness_support_client = foreground
                    .spawn(|_, ctx| ServerApiProvider::as_ref(ctx).get_harness_support_client())
                    .await?;
                let resume_conversation_id = AIConversationId::try_from(conversation_id.clone())
                    .map_err(|err| AgentDriverError::ConversationLoadFailed(format!("{err:#}")))?;
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

    /// Resolve the environment and store into `driver_options`.
    async fn resolve_environment(
        foreground: &ModelSpawner<Self>,
        environment_id: Option<String>,
        driver_options: &mut AgentDriverOptions,
    ) -> Result<(), AgentDriverError> {
        let Some(environment_id) = environment_id else {
            return Ok(());
        };

        let environment = foreground
            .spawn(move |_, ctx| -> Result<_, AgentDriverError> {
                let server_id = ServerId::try_from(environment_id.as_str()).map_err(|_| {
                    log::error!("Invalid environment ID: {environment_id}");
                    AgentDriver::log_valid_environments(ctx);
                    AgentDriverError::EnvironmentNotFound(environment_id.clone())
                })?;
                let sync_id = SyncId::ServerId(server_id);

                CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx)
                    .ok_or_else(|| {
                        log::error!("Environment not found with ID: {environment_id}");
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

        driver_options.environment = Some(environment);
        Ok(())
    }

    /// Create the AgentDriver and start running the task.
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
            let agent_future = driver.run(task, ctx);

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
        CliCommand::Run(task_cmd) => match task_cmd {
            TaskCommand::List { .. } => true,
            TaskCommand::Get { .. } => true,
            TaskCommand::Conversation { .. } => true,
            TaskCommand::Message { .. } => true,
        },
        CliCommand::Model(model_cmd) => match model_cmd {
            ModelCommand::List => true,
        },
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

    // Trigger the user refresh - the subscription above will handle the result.
    AuthManager::handle(ctx).update(ctx, |auth_manager, ctx| {
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
        CliCommand::Model(ModelCommand::List) => CliTelemetryEvent::ModelList,
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
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
