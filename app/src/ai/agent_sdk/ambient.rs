//! Commands to interact with ambient agents on Warp's platform.
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _};
use comfy_table::Cell;
use futures::{future, StreamExt};
use serde::Serialize;
use warp_cli::agent::{Harness, OutputFormat, Prompt, RunCloudArgs};
use warp_cli::json_filter::JsonOutput;
use warp_cli::task::{
    ArtifactTypeArg, ExecutionLocationArg, ListTasksArgs, MessageCommand, MessageDeliveredArgs,
    MessageListArgs, MessageReadArgs, MessageSendArgs, MessageWatchArgs, RunSortByArg,
    RunSourceArg, RunStateArg, TaskGetArgs,
};
use warp_cli::{GlobalOptions, SortOrderArg};
use warp_core::channel::ChannelState;
use warp_core::features::FeatureFlag;
use warpui::platform::TerminationMode;
use warpui::r#async::{Spawnable, Timer};
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::common::{parse_ambient_task_id, EnvironmentChoice, ResolveConfigurationError};
use crate::ai::agent::{extract_user_query_mode, UserQueryMode};
use crate::ai::agent_sdk::driver::attachments::{
    process_attachment, MAX_ATTACHMENT_COUNT_FOR_CLOUD_QUERY,
};
use crate::ai::ambient_agents::spawn::{
    spawn_task, AmbientAgentEvent, SessionJoinInfo, TASK_STATUS_POLLING_DURATION,
};
use crate::ai::ambient_agents::task::HarnessConfig;
use crate::ai::ambient_agents::{
    AgentConfigSnapshot, AmbientAgentTask, AmbientAgentTaskId, AmbientAgentTaskState,
};
use crate::ai::artifacts::Artifact;
use crate::auth::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::server::ids::{ServerId, SyncId};
use crate::server::server_api::ai::{
    AIClient, AgentMessageHeader, AgentRunEvent, AgentSource, ArtifactType, ExecutionLocation,
    ListAgentMessagesRequest, ReadAgentMessageResponse, RunSortBy, RunSortOrder,
    SendAgentMessageRequest, SendAgentMessageResponse, SpawnAgentRequest, TaskListFilter,
};
use crate::server::server_api::ServerApi;
use crate::terminal::shared_session;
use crate::util::time_format::format_approx_duration_from_now_utc;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::ServerApiProvider;

const MAX_LINE_WIDTH: usize = 90;
const STREAM_RETRY_BACKOFF_STEPS: &[u64] = &[1, 2, 5, 10];

/// Singleton model that runs async work for ambient agent CLI commands.
struct AmbientAgentRunner;

/// Run an ambient agent with the provided arguments.
pub fn run_ambient_agent(ctx: &mut AppContext, args: RunCloudArgs) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AmbientAgentRunner);
    runner.update(ctx, |runner, ctx| runner.run_agent(args, ctx))
}

/// List ambient agent tasks.
pub fn list_ambient_agent_tasks(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    args: ListTasksArgs,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AmbientAgentRunner);
    let filter = filter_from_args(&args);
    let json_output = args.json_output.clone();
    let output_format = global_options.output_format;
    runner.update(ctx, |runner, ctx| {
        runner.list_tasks(args.limit, filter, output_format, json_output, ctx)
    })
}

/// Print a table of ambient agent tasks.
pub(super) fn print_tasks(tasks: &[AmbientAgentTask]) {
    AmbientAgentRunner::print_tasks_table(tasks);
}

/// Get status of a specific ambient agent task.
pub fn get_ambient_agent_task_status(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    args: TaskGetArgs,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AmbientAgentRunner);
    let output_format = global_options.output_format;
    runner.update(ctx, |runner, ctx| {
        runner.get_task_status(args, output_format, ctx)
    })
}

/// Translate CLI-level `ListTasksArgs` into the server-facing `TaskListFilter`.
pub(super) fn filter_from_args(args: &ListTasksArgs) -> TaskListFilter {
    let states = if args.state.is_empty() {
        None
    } else {
        Some(
            args.state
                .iter()
                .map(|s| run_state_from_arg(*s))
                .collect::<Vec<_>>(),
        )
    };

    TaskListFilter {
        creator_uid: args.creator.clone(),
        updated_after: args.updated_after,
        created_after: args.created_after,
        created_before: args.created_before,
        states,
        source: args.source.map(run_source_from_arg),
        execution_location: args.execution_location.map(execution_location_from_arg),
        environment_id: args.environment.clone(),
        skill_spec: args.skill.clone(),
        schedule_id: args.schedule.clone(),
        ancestor_run_id: args.ancestor_run.clone(),
        config_name: args.name.clone(),
        model_id: args.model.clone(),
        artifact_type: args.artifact_type.map(artifact_type_from_arg),
        search_query: args.query.clone(),
        sort_by: args.sort_by.map(sort_by_from_arg),
        sort_order: args.sort_order.map(sort_order_from_arg),
        cursor: args.cursor.clone(),
    }
}

fn run_state_from_arg(arg: RunStateArg) -> AmbientAgentTaskState {
    match arg {
        RunStateArg::Queued => AmbientAgentTaskState::Queued,
        RunStateArg::Pending => AmbientAgentTaskState::Pending,
        RunStateArg::Claimed => AmbientAgentTaskState::Claimed,
        RunStateArg::InProgress => AmbientAgentTaskState::InProgress,
        RunStateArg::Succeeded => AmbientAgentTaskState::Succeeded,
        RunStateArg::Failed => AmbientAgentTaskState::Failed,
        RunStateArg::Error => AmbientAgentTaskState::Error,
        RunStateArg::Blocked => AmbientAgentTaskState::Blocked,
        RunStateArg::Cancelled => AmbientAgentTaskState::Cancelled,
    }
}

fn run_source_from_arg(arg: RunSourceArg) -> AgentSource {
    match arg {
        RunSourceArg::Api => AgentSource::AgentWebhook,
        RunSourceArg::Cli => AgentSource::Cli,
        RunSourceArg::Slack => AgentSource::Slack,
        RunSourceArg::Linear => AgentSource::Linear,
        RunSourceArg::ScheduledAgent => AgentSource::ScheduledAgent,
        RunSourceArg::WebApp => AgentSource::WebApp,
        RunSourceArg::CloudMode => AgentSource::CloudMode,
        RunSourceArg::GitHubAction => AgentSource::GitHubAction,
        RunSourceArg::Interactive => AgentSource::Interactive,
    }
}

fn execution_location_from_arg(arg: ExecutionLocationArg) -> ExecutionLocation {
    match arg {
        ExecutionLocationArg::Local => ExecutionLocation::Local,
        ExecutionLocationArg::Remote => ExecutionLocation::Remote,
    }
}

fn artifact_type_from_arg(arg: ArtifactTypeArg) -> ArtifactType {
    match arg {
        ArtifactTypeArg::Plan => ArtifactType::Plan,
        ArtifactTypeArg::PullRequest => ArtifactType::PullRequest,
        ArtifactTypeArg::Screenshot => ArtifactType::Screenshot,
        ArtifactTypeArg::File => ArtifactType::File,
    }
}

fn sort_by_from_arg(arg: RunSortByArg) -> RunSortBy {
    match arg {
        RunSortByArg::UpdatedAt => RunSortBy::UpdatedAt,
        RunSortByArg::CreatedAt => RunSortBy::CreatedAt,
        RunSortByArg::Title => RunSortBy::Title,
        RunSortByArg::Agent => RunSortBy::Agent,
    }
}

fn sort_order_from_arg(arg: SortOrderArg) -> RunSortOrder {
    match arg {
        SortOrderArg::Asc => RunSortOrder::Asc,
        SortOrderArg::Desc => RunSortOrder::Desc,
    }
}

/// Run a message-related CLI command.
pub fn run_message(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: MessageCommand,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AmbientAgentRunner);
    let output_format = global_options.output_format;
    match command {
        MessageCommand::Watch(args) => runner.update(ctx, |runner, ctx| {
            runner.watch_messages(args, output_format, ctx)
        }),
        MessageCommand::Send(args) => runner.update(ctx, |runner, ctx| {
            runner.send_message(args, output_format, ctx)
        }),
        MessageCommand::List(args) => runner.update(ctx, |runner, ctx| {
            runner.list_messages(args, output_format, ctx)
        }),
        MessageCommand::Read(args) => runner.update(ctx, |runner, ctx| {
            runner.read_message(args, output_format, ctx)
        }),
        MessageCommand::MarkDelivered(args) => runner.update(ctx, |runner, ctx| {
            runner.mark_message_delivered(args, output_format, ctx)
        }),
    }
}

impl AmbientAgentRunner {
    fn spawn_command(
        &self,
        future: impl Spawnable<Output = anyhow::Result<()>>,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.spawn(future, |_, result, ctx| match result {
            Ok(()) => {
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            Err(err) => {
                super::report_fatal_error(err, ctx);
            }
        });
    }
    fn run_agent(&self, args: RunCloudArgs, ctx: &mut ModelContext<Self>) -> anyhow::Result<()> {
        if !FeatureFlag::AmbientAgentsCommandLine.is_enabled() {
            return Err(anyhow::anyhow!(i18n::t(
                "ai.agent_sdk.ambient.unsupported_feature"
            )));
        }
        let skill_enabled = FeatureFlag::OzPlatformSkills.is_enabled();
        if args.skill.is_some() && !skill_enabled {
            return Err(anyhow::anyhow!(i18n::t(
                "ai.agent_sdk.ambient.unexpected_skill_arg"
            )));
        }

        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        let setup_future = future::try_join(refresh_future, warp_drive_sync_future);

        ctx.spawn(setup_future, move |_runner, setup_result, ctx| {
            if let Err(err) = setup_result {
                super::report_fatal_error(err, ctx);
                return;
            }

            // Validate that at least one of prompt, skill, or conversation is provided.
            // conversation is used to continue an existing cloud conversation.
            let prompt = args.prompt_arg.to_prompt();
            let has_prompt_source = prompt.is_some()
                || (skill_enabled && args.skill.is_some())
                || args.conversation.is_some();
            if !has_prompt_source {
                super::report_fatal_error(
                    anyhow::anyhow!(i18n::t("ai.agent_sdk.ambient.missing_prompt_source")),
                    ctx,
                );
                return;
            }
            let prompt = match prompt {
                Some(Prompt::PlainText(text)) => Some(text),
                Some(Prompt::SavedPrompt(id)) => {
                    // Resolve the saved prompt to pass along as the ambient agent query.
                    // We look up the prompt text here, rather than passing along the saved prompt ID,
                    // in order to support personal saved prompts, which team service accounts would not
                    // have access to.
                    // TODO: we should pipe the saved prompt ID through the API, and resolve it server-side.
                    // That'd also allow finding all tasks which used a given saved prompt.
                    let sync_id: SyncId = match ServerId::try_from(id.as_str()) {
                        Ok(server_id) => server_id.into(),
                        Err(err) => {
                            super::report_fatal_error(
                                anyhow::anyhow!(i18n::t(
                                    "ai.agent_sdk.ambient.saved_prompt_parse_failed"
                                )
                                .replace("{id}", &id)
                                .replace("{error}", &err.to_string())),
                                ctx,
                            );
                            return;
                        }
                    };

                    let cloud_model = CloudModel::handle(ctx);
                    let workflow = cloud_model.as_ref(ctx).get_workflow(&sync_id);

                    match workflow {
                        Some(cloud_workflow) => match cloud_workflow.model().data.prompt() {
                            Some(prompt_text) => Some(prompt_text.to_string()),
                            None => {
                                super::report_fatal_error(
                                    anyhow::anyhow!(i18n::t(
                                        "ai.agent_sdk.ambient.saved_prompt_not_prompt"
                                    )
                                    .replace("{id}", &id)),
                                    ctx,
                                );
                                return;
                            }
                        },
                        None => {
                            super::report_fatal_error(
                                anyhow::anyhow!(i18n::t(
                                    "ai.agent_sdk.ambient.saved_prompt_not_found"
                                )
                                .replace("{id}", &id)),
                                ctx,
                            );
                            return;
                        }
                    }
                }
                None => None,
            };

            let loaded_file = match args.config_file.file.as_deref() {
                Some(path) => match super::config_file::load_config_file(path) {
                    Ok(file) => Some(file),
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                },
                None => None,
            };

            // Validate and process attachments early, before environment selection
            // This ensures users don't have to go through env selection if attachment validation fails
            if args.attachment_paths.len() > MAX_ATTACHMENT_COUNT_FOR_CLOUD_QUERY {
                super::report_fatal_error(
                    anyhow::anyhow!(i18n::t("ai.agent_sdk.ambient.too_many_attachments")
                        .replace("{max}", &MAX_ATTACHMENT_COUNT_FOR_CLOUD_QUERY.to_string())
                        .replace("{count}", &args.attachment_paths.len().to_string())),
                    ctx,
                );
                return;
            }

            let attachments = if FeatureFlag::AmbientAgentsImageUpload.is_enabled() {
                if !args.attachment_paths.is_empty() {
                    match args
                        .attachment_paths
                        .iter()
                        .enumerate()
                        .map(|(i, path)| process_attachment(path, i))
                        .collect::<Result<Vec<_>, _>>()
                    {
                        Ok(processed) => processed,
                        Err(err) => {
                            super::report_fatal_error(err, ctx);
                            return;
                        }
                    }
                } else {
                    vec![]
                }
            } else {
                if !args.attachment_paths.is_empty() {
                    super::report_fatal_error(
                        anyhow::anyhow!(i18n::t(
                            "ai.agent_sdk.ambient.attachment_upload_not_enabled"
                        )),
                        ctx,
                    );
                    return;
                }
                vec![]
            };

            let mut environment_args = args.environment;
            if environment_args.environment.is_none() && !environment_args.no_environment {
                if let Some(environment_id) = loaded_file
                    .as_ref()
                    .and_then(|f| f.file.environment_id.clone())
                {
                    environment_args.environment = Some(environment_id);
                }
            }

            let environment_id = match EnvironmentChoice::resolve_for_create(environment_args, ctx)
            {
                Ok(EnvironmentChoice::None) => {
                    eprintln!("{}", i18n::t("ai.agent_sdk.ambient.without_environment"));
                    None
                }
                Ok(EnvironmentChoice::Environment { id, .. }) => Some(id),
                Err(ResolveConfigurationError::Canceled) => {
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                    return;
                }
                Err(err) => {
                    super::report_fatal_error(anyhow::anyhow!(err), ctx);
                    return;
                }
            };

            let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

            // Compute the upgrade link in case we hit capacity.
            let upgrade_link = AuthStateProvider::as_ref(ctx)
                .get()
                .user_id()
                .map(UserWorkspaces::upgrade_link);

            let cli_mcp_servers =
                match super::mcp_config::build_mcp_servers_from_specs(&args.mcp_specs) {
                    Ok(mcp_servers) => mcp_servers,
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                };

            let harness_override = (args.harness != Harness::Oz).then_some(HarnessConfig {
                harness_type: args.harness,
                model_id: None,
                reasoning_level: None,
            });
            let harness_auth_secrets = args.claude_auth_secret.clone().map(|name| {
                crate::ai::ambient_agents::task::HarnessAuthSecretsConfig {
                    claude_auth_secret_name: Some(name),
                    codex_auth_secret_name: None,
                }
            });

            let merged_config = super::config_file::merge_with_precedence(
                loaded_file.as_ref(),
                AgentConfigSnapshot {
                    name: args.name,
                    environment_id,
                    model_id: args.model.model.clone(),
                    base_prompt: None,
                    mcp_servers: cli_mcp_servers,
                    profile_id: None,
                    worker_host: args.worker_host.clone(),
                    skill_spec: None,
                    computer_use_enabled: args.computer_use.computer_use_override(),
                    harness: harness_override,
                    harness_auth_secrets,
                },
            );

            // We must wait until after workspace metadata is refreshed to check available LLMs.
            let model_id = match merged_config
                .model_id
                .as_deref()
                .map(|model_id| super::common::validate_agent_mode_base_model_id(model_id, ctx))
                .transpose()
            {
                Ok(id) => id.map(|id| id.to_string()),
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                    return;
                }
            };

            let config = {
                let mut config = merged_config;
                config.model_id = model_id;
                if config.is_empty() {
                    None
                } else {
                    Some(config)
                }
            };

            // For ambient runs, skill is passed to the server and resolved in the remote environment
            let skill = if skill_enabled {
                args.skill.as_ref().map(|s| s.to_string())
            } else {
                None
            };

            let (prompt, mode) = match prompt {
                Some(prompt) => {
                    let (prompt, mode) = extract_user_query_mode(prompt);
                    (Some(prompt), mode)
                }
                None => (None, UserQueryMode::Normal),
            };
            let request = SpawnAgentRequest {
                prompt,
                mode,
                config,
                title: None,
                team: match (args.scope.team, args.scope.personal) {
                    (true, _) => Some(true),
                    (_, true) => Some(false),
                    _ => None,
                },
                agent_identity_uid: args.agent_uid,
                skill,
                attachments,
                interactive: None,
                parent_run_id: None,
                runtime_skills: vec![],
                referenced_attachments: vec![],
                conversation_id: args.conversation,
                initial_snapshot_token: None,
                snapshot_disabled: None,
            };

            let should_open = args.open;
            let oz_root_url = ChannelState::oz_root_url();
            let ai_client_clone = ai_client.clone();
            let spawn_future = async move {
                let mut stream = Box::pin(spawn_task(
                    request,
                    ai_client_clone,
                    Some(TASK_STATUS_POLLING_DURATION),
                ));
                let mut session_join_info = None;
                let mut spawned_task_id = None;

                while let Some(event_result) = stream.next().await {
                    match event_result {
                        Ok(event) => match event {
                            AmbientAgentEvent::TaskSpawned { task_id, .. } => {
                                println!(
                                    "{}",
                                    i18n::t("ai.agent_sdk.ambient.spawned")
                                        .replace("{id}", &task_id.to_string())
                                );
                                println!(
                                    "{}",
                                    i18n::t("ai.agent_sdk.ambient.view_run")
                                        .replace("{url}", &format!("{oz_root_url}/runs/{task_id}"))
                                );
                                spawned_task_id = Some(task_id);
                            }
                            AmbientAgentEvent::AtCapacity => {
                                println!(
                                    "{}",
                                    i18n::t("ai.agent_sdk.ambient.concurrent_limit_reached")
                                );
                                if let Some(url) = &upgrade_link {
                                    println!(
                                        "{}",
                                        i18n::t("ai.agent_sdk.ambient.upgrade_plan")
                                            .replace("{url}", url)
                                    );
                                }
                            }
                            AmbientAgentEvent::StateChanged {
                                state,
                                status_message,
                            } => {
                                if matches!(
                                    state,
                                    AmbientAgentTaskState::InProgress
                                        | AmbientAgentTaskState::Succeeded
                                ) || state.is_failure_like()
                                {
                                    println!(
                                        "{}",
                                        i18n::t("ai.agent_sdk.ambient.agent_state")
                                            .replace("{state}", &format!("{state:?}"))
                                    );
                                }
                                if state.is_failure_like() {
                                    if let Some(msg) = status_message {
                                        println!(
                                            "{}",
                                            i18n::t("ai.agent_sdk.ambient.error")
                                                .replace("{message}", &msg.message)
                                        );
                                    } else {
                                        println!(
                                            "{}",
                                            i18n::t("ai.agent_sdk.ambient.failed_no_error_message")
                                        );
                                    }
                                }
                            }
                            AmbientAgentEvent::SessionStarted {
                                session_join_info: info,
                            } => {
                                println!(
                                    "{}",
                                    i18n::t("ai.agent_sdk.ambient.view_session")
                                        .replace("{url}", &info.session_link)
                                );
                                session_join_info = Some(info);
                            }
                            AmbientAgentEvent::TimedOut => {
                                let task_id_str = spawned_task_id.as_ref().map_or_else(
                                    || i18n::t("ai.agent_sdk.ambient.unknown"),
                                    |id| id.to_string(),
                                );
                                println!(
                                    "{}",
                                    i18n::t("ai.agent_sdk.ambient.session_not_ready")
                                        .replace("{id}", &task_id_str)
                                        .replace(
                                            "{seconds}",
                                            &TASK_STATUS_POLLING_DURATION.as_secs().to_string(),
                                        )
                                );
                            }
                        },
                        Err(err) => {
                            return Err(err);
                        }
                    }
                }

                Ok(session_join_info)
            };

            ctx.spawn(spawn_future, move |_, result, ctx| match result {
                Ok(session_join_info) => {
                    if should_open {
                        if let Some(session_join_info) = session_join_info {
                            let url =
                                match (super::is_running_in_warp(), session_join_info.session_id) {
                                    (true, Some(session_id)) => {
                                        shared_session::join_native_intent(&session_id)
                                    }
                                    _ => session_join_info.session_link,
                                };

                            ctx.open_url(&url);
                        }
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            });
        });

        Ok(())
    }

    fn list_tasks(
        &self,
        limit: i32,
        filter: TaskListFilter,
        output_format: OutputFormat,
        json_output: JsonOutput,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        let list_future = async move {
            if matches!(output_format, OutputFormat::Json) || json_output.force_json_output() {
                let response = ai_client.list_agent_runs_raw(limit, filter).await?;
                super::output::print_raw_json(response, &json_output)?;
            } else if matches!(output_format, OutputFormat::Ndjson) {
                let tasks = ai_client.list_ambient_agent_tasks(limit, filter).await?;
                for task in tasks {
                    super::output::write_json_line(&task, std::io::stdout())?;
                }
            } else {
                let tasks = ai_client.list_ambient_agent_tasks(limit, filter).await?;
                Self::print_tasks_table(&tasks);
            }
            Ok(())
        };
        self.spawn_command(list_future, ctx);

        Ok(())
    }

    fn get_task_status(
        &self,
        args: TaskGetArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        let status_future = async move {
            let task_id = args.task_id.parse()?;
            let json_output = args.json_output;
            if matches!(output_format, OutputFormat::Json) || json_output.force_json_output() {
                let response = ai_client.get_agent_run_raw(&task_id).await?;
                super::output::print_raw_json(response, &json_output)?;
            } else if matches!(output_format, OutputFormat::Ndjson) {
                let task = ai_client.get_ambient_agent_task(&task_id).await?;
                super::output::write_json_line(&task, std::io::stdout())?;
            } else {
                let task = ai_client.get_ambient_agent_task(&task_id).await?;
                Self::print_tasks_table(&[task]);
            }
            Ok(())
        };
        self.spawn_command(status_future, ctx);

        Ok(())
    }

    fn send_message(
        &self,
        args: MessageSendArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let provider = ServerApiProvider::as_ref(ctx);
        let ai_client = provider.get_ai_client();
        let server_api = provider.get();
        let scoped_task_id = task_id_for_message_send(&args.sender_run_id)?;

        let future = async move {
            let request = SendAgentMessageRequest {
                to: args.to,
                subject: args.subject,
                body: args.body,
                sender_run_id: args.sender_run_id,
            };
            let log_context = SendAgentMessageLogContext::new(&request, scoped_task_id.as_ref());
            log_context.log_start();
            let send_message = async move {
                match scoped_task_id {
                    Some(task_id) => {
                        server_api
                            .send_agent_message_for_task(&task_id, request)
                            .await
                    }
                    None => ai_client.send_agent_message(request).await,
                }
            };
            let response = match send_message.await {
                Ok(response) => {
                    log_context.log_success(&response);
                    response
                }
                Err(err) => {
                    let err = err.context(log_context.error_context());
                    log_context.log_error(&err);
                    eprintln!("{err:#}");
                    return Err(err);
                }
            };
            print_send_message_response(&response, output_format)?;
            Ok(())
        };
        self.spawn_command(future, ctx);

        Ok(())
    }

    fn list_messages(
        &self,
        args: MessageListArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let provider = ServerApiProvider::as_ref(ctx);
        let ai_client = provider.get_ai_client();
        let server_api = provider.get();

        let future = async move {
            let request = ListAgentMessagesRequest {
                unread_only: args.unread,
                since: args.since,
                limit: args.limit,
            };
            let messages = match task_id_from_run_id(&args.run_id) {
                Some(task_id) => {
                    server_api
                        .list_agent_messages_for_task(&task_id, &args.run_id, request)
                        .await?
                }
                None => ai_client.list_agent_messages(&args.run_id, request).await?,
            };
            super::output::print_list(messages, output_format);
            Ok(())
        };
        self.spawn_command(future, ctx);

        Ok(())
    }

    fn watch_messages(
        &self,
        args: MessageWatchArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        ensure_stream_output_format(output_format)?;
        let provider = ServerApiProvider::as_ref(ctx);
        let server_api = provider.get();
        let ai_client = provider.get_ai_client();

        let future = async move { watch_messages_forever(server_api, ai_client, args).await };
        self.spawn_command(future, ctx);

        Ok(())
    }

    fn read_message(
        &self,
        args: MessageReadArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let provider = ServerApiProvider::as_ref(ctx);
        let ai_client = provider.get_ai_client();
        let server_api = provider.get();
        let scoped_task_id = task_id_from_oz_run_id_env()?;

        let future = async move {
            let message = match scoped_task_id {
                Some(task_id) => {
                    server_api
                        .read_agent_message_for_task(&task_id, &args.message_id)
                        .await?
                }
                None => ai_client.read_agent_message(&args.message_id).await?,
            };
            print_read_message_response(&message, output_format)?;
            Ok(())
        };
        self.spawn_command(future, ctx);

        Ok(())
    }

    fn mark_message_delivered(
        &self,
        args: MessageDeliveredArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let provider = ServerApiProvider::as_ref(ctx);
        let ai_client = provider.get_ai_client();
        let server_api = provider.get();
        let scoped_task_id = task_id_from_oz_run_id_env()?;

        let future = async move {
            match scoped_task_id {
                Some(task_id) => {
                    server_api
                        .mark_message_delivered_for_task(&task_id, &args.message_id)
                        .await?
                }
                None => ai_client.mark_message_delivered(&args.message_id).await?,
            }
            print_mark_message_delivered_result(&args.message_id, output_format)?;
            Ok(())
        };
        self.spawn_command(future, ctx);

        Ok(())
    }

    /// Get the appropriate emoji for a task state.
    fn get_state_emoji(state: &AmbientAgentTaskState) -> &'static str {
        match state {
            AmbientAgentTaskState::Queued | AmbientAgentTaskState::Pending => "⏳",
            AmbientAgentTaskState::Claimed => "🔄",
            AmbientAgentTaskState::InProgress => "🔄",
            AmbientAgentTaskState::Succeeded => "✅",
            AmbientAgentTaskState::Failed
            | AmbientAgentTaskState::Error
            | AmbientAgentTaskState::Unknown => "❌",
            AmbientAgentTaskState::Blocked => "🛑",
            AmbientAgentTaskState::Cancelled => "🚫",
        }
    }

    /// Print runs in a beautifully formatted ASCII table with card-style layout.
    fn print_tasks_table(tasks: &[AmbientAgentTask]) {
        if tasks.is_empty() {
            println!("{}", i18n::t("ai.agent_sdk.ambient.no_runs_found"));
            return;
        }

        if tasks.len() == 1 {
            println!("\n{}", i18n::t("ai.agent_sdk.ambient.heading.run"));
        } else {
            println!(
                "\n{}",
                i18n::t("ai.agent_sdk.ambient.heading.runs")
                    .replace("{count}", &tasks.len().to_string())
            );
        }

        let oz_root_url = ChannelState::oz_root_url();
        for task in tasks {
            let state_emoji = Self::get_state_emoji(&task.state);

            // Create a single-column table for each run (card-style)
            let mut table = crate::ai::agent_sdk::output::standard_table();

            // Run header with emoji and ID
            let header = format!("{} {} ({:?})", state_emoji, task.task_id, task.state);
            table.add_row(vec![header]);

            // Oz webapp link
            table.add_row(vec![format!("Oz: {oz_root_url}/runs/{}", task.task_id)]);

            // Title (wrapped, single cell)
            if !task.title.is_empty() {
                let title_label = i18n::t("ai.agent_sdk.ambient.label.title");
                let title_cell = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
                    &title_label,
                    &task.title,
                    MAX_LINE_WIDTH,
                );
                table.add_row(vec![title_cell]);
            }

            if let Some(executor) = task.executor_display_name() {
                table
                    .add_row(vec![i18n::t("ai.agent_sdk.ambient.executed_as")
                        .replace("{executor}", &executor)]);
            }

            // Agent config snapshot (if available)
            if let Some(config) = task.agent_config_snapshot.as_ref() {
                let config_str =
                    serde_json::to_string_pretty(config).unwrap_or_else(|_| format!("{config:?}"));
                table.add_row(vec![
                    i18n::t("ai.agent_sdk.ambient.config").replace("{config}", &config_str)
                ]);
            }

            // Created time
            let created_formatted = format_approx_duration_from_now_utc(task.created_at);
            table.add_row(vec![
                i18n::t("ai.agent_sdk.ambient.created").replace("{time}", &created_formatted)
            ]);

            // Status message (if available) - single multi-line cell
            if let Some(status_msg) = &task.status_message {
                let status_label = i18n::t("ai.agent_sdk.ambient.label.status");
                let status_cell = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
                    &status_label,
                    &status_msg.message,
                    MAX_LINE_WIDTH,
                );
                table.add_row(vec![status_cell]);
            }

            // Artifacts (if available)
            if !task.artifacts.is_empty() {
                let artifacts_cell = Self::format_artifacts(&task.artifacts);
                table.add_row(vec![artifacts_cell]);
            }

            // Session link (if available)
            if let Some(session_join_info) = SessionJoinInfo::from_task(task) {
                table.add_row(vec![i18n::t("ai.agent_sdk.ambient.session")
                    .replace("{url}", &session_join_info.session_link)]);
            }

            println!("{table}");
        }
    }

    /// Format artifacts for display.
    fn format_artifacts(artifacts: &[Artifact]) -> String {
        let mut lines = vec![i18n::t("ai.agent_sdk.ambient.artifacts")];

        for artifact in artifacts {
            match artifact {
                Artifact::PullRequest {
                    url,
                    branch,
                    repo,
                    number,
                    ..
                } => {
                    let pr_display = match (repo, number) {
                        (Some(repo), Some(num)) => {
                            format!(
                                "  {}",
                                i18n::t("ai.agent_sdk.ambient.artifact.pr_with_repo")
                                    .replace("{repo}", repo)
                                    .replace("{number}", &num.to_string())
                            )
                        }
                        _ => format!("  {}", i18n::t("ai.agent_sdk.ambient.artifact.pr")),
                    };
                    lines.push(pr_display);
                    lines.push(format!(
                        "    {}",
                        i18n::t("ai.agent_sdk.ambient.artifact.branch").replace("{branch}", branch)
                    ));
                    lines.push(format!(
                        "    {}",
                        i18n::t("ai.agent_sdk.ambient.artifact.link").replace("{url}", url)
                    ));
                }
                Artifact::Plan {
                    notebook_uid,
                    title,
                    ..
                } => {
                    let plan_title = title
                        .as_deref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| i18n::t("ai.agent_sdk.ambient.artifact.untitled_plan"));
                    lines.push(format!(
                        "  {}",
                        i18n::t("ai.agent_sdk.ambient.artifact.plan")
                            .replace("{title}", &plan_title)
                    ));
                    if let Some(id) = notebook_uid {
                        lines.push(format!(
                            "    {}",
                            i18n::t("ai.agent_sdk.ambient.artifact.link").replace(
                                "{url}",
                                &format!(
                                    "{}/drive/notebook/{}",
                                    ChannelState::server_root_url(),
                                    id
                                ),
                            )
                        ));
                    }
                }
                Artifact::Screenshot {
                    artifact_uid,
                    description,
                    ..
                } => {
                    let desc = description
                        .as_deref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| i18n::t("ai.agent_sdk.ambient.artifact.no_description"));
                    lines.push(format!(
                        "  {}",
                        i18n::t("ai.agent_sdk.ambient.artifact.screenshot")
                            .replace("{uid}", artifact_uid)
                            .replace("{description}", &desc)
                    ));
                }
                Artifact::File {
                    filename,
                    filepath,
                    description,
                    ..
                } => {
                    let label = super::super::artifacts::file_button_label(filename, filepath);
                    lines.push(format!(
                        "  {}",
                        i18n::t("ai.agent_sdk.ambient.artifact.file").replace("{label}", &label)
                    ));
                    lines.push(format!(
                        "    {}",
                        i18n::t("ai.agent_sdk.ambient.artifact.path").replace("{path}", filepath)
                    ));
                    if let Some(description) = description {
                        lines.push(format!(
                            "    {}",
                            i18n::t("ai.agent_sdk.ambient.artifact.description")
                                .replace("{description}", description)
                        ));
                    }
                }
            }
        }

        lines.join("\n")
    }
}

#[derive(Serialize)]
struct MessageDeliveredResult<'a> {
    message_id: &'a str,
    delivered: bool,
}

#[derive(Serialize)]
struct MessageWatchEvent {
    sequence: i64,
    message_id: String,
    sender_run_id: String,
    subject: String,
    body: String,
    occurred_at: String,
}

fn format_optional_timestamp(timestamp: Option<&str>) -> &str {
    timestamp.unwrap_or("-")
}

fn ensure_stream_output_format(output_format: OutputFormat) -> anyhow::Result<()> {
    if output_format == OutputFormat::Ndjson {
        return Ok(());
    }

    Err(anyhow!(i18n::t(
        "ai.agent_sdk.ambient.streaming_requires_ndjson"
    )))
}

fn stream_retry_backoff(failures: usize) -> Duration {
    let index = failures
        .saturating_sub(1)
        .min(STREAM_RETRY_BACKOFF_STEPS.len() - 1);
    Duration::from_secs(STREAM_RETRY_BACKOFF_STEPS[index])
}

fn write_stream_record<T: Serialize>(record: &T) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout();
    super::output::write_json_line(record, &mut stdout)?;
    stdout
        .flush()
        .context(i18n::t("ai.agent_sdk.ambient.flush_stdout_failed"))?;
    Ok(())
}

fn task_id_from_run_id(run_id: &str) -> Option<AmbientAgentTaskId> {
    run_id.parse().ok()
}

fn task_id_from_oz_run_id_env() -> anyhow::Result<Option<AmbientAgentTaskId>> {
    match std::env::var(warp_cli::OZ_RUN_ID_ENV) {
        Ok(run_id) => {
            let prefix = i18n::t("ai.agent_sdk.ambient.invalid_oz_run_id");
            parse_ambient_task_id(&run_id, &prefix).map(Some)
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(anyhow!(i18n::t("ai.agent_sdk.ambient.env_not_unicode")
                .replace("{env}", warp_cli::OZ_RUN_ID_ENV)))
        }
    }
}

fn task_id_for_message_send(sender_run_id: &str) -> anyhow::Result<Option<AmbientAgentTaskId>> {
    match task_id_from_run_id(sender_run_id) {
        Some(task_id) => Ok(Some(task_id)),
        None => task_id_from_oz_run_id_env(),
    }
}

#[derive(Debug, Clone)]
struct SendAgentMessageLogContext {
    sender_run_id: String,
    task_id: Option<String>,
    target_agent_ids: Vec<String>,
    subject: String,
    body_len: usize,
}

impl SendAgentMessageLogContext {
    fn new(request: &SendAgentMessageRequest, task_id: Option<&AmbientAgentTaskId>) -> Self {
        Self {
            sender_run_id: request.sender_run_id.clone(),
            task_id: task_id.map(|task_id| task_id.to_string()),
            target_agent_ids: request.to.clone(),
            subject: request.subject.clone(),
            body_len: request.body.chars().count(),
        }
    }

    fn error_context(&self) -> String {
        let sender_run_id = format!("{:?}", self.sender_run_id);
        let task_id = format!("{:?}", self.task_id);
        let target_agent_ids = format!("{:?}", self.target_agent_ids);
        i18n::t("ai.agent_sdk.ambient.message.send_failed_context")
            .replace("{sender_run_id}", &sender_run_id)
            .replace("{task_id}", &task_id)
            .replace("{target_agent_ids}", &target_agent_ids)
    }

    fn log_start(&self) {
        log::info!(
            "Sending ambient agent message: sender_run_id={:?} task_id={:?} target_agent_ids={:?} subject={:?} body_len={}",
            self.sender_run_id,
            self.task_id,
            self.target_agent_ids,
            self.subject,
            self.body_len
        );
    }

    fn log_success(&self, response: &SendAgentMessageResponse) {
        log::info!(
            "Sent ambient agent message: sender_run_id={:?} task_id={:?} target_agent_ids={:?} subject={:?} body_len={} message_ids={:?}",
            self.sender_run_id,
            self.task_id,
            self.target_agent_ids,
            self.subject,
            self.body_len,
            response.message_ids
        );
    }

    fn log_error(&self, err: &anyhow::Error) {
        log::warn!(
            "Failed to send ambient agent message: sender_run_id={:?} task_id={:?} target_agent_ids={:?} subject={:?} body_len={} error={err:#}",
            self.sender_run_id,
            self.task_id,
            self.target_agent_ids,
            self.subject,
            self.body_len
        );
    }
}

async fn watch_messages_forever(
    server_api: Arc<ServerApi>,
    ai_client: Arc<dyn AIClient>,
    args: MessageWatchArgs,
) -> anyhow::Result<()> {
    let run_id = args.run_id;
    let watched_run_ids = vec![run_id.clone()];
    let scoped_task_id = task_id_from_run_id(&run_id);
    let mut last_seen_sequence = args.since_sequence;
    let mut initial_connect = true;
    let mut failures = 0usize;

    loop {
        let stream_result = match scoped_task_id.as_ref() {
            Some(task_id) => {
                server_api
                    .stream_agent_events_for_task(task_id, &watched_run_ids, last_seen_sequence)
                    .await
            }
            None => {
                server_api
                    .stream_agent_events(&watched_run_ids, last_seen_sequence)
                    .await
            }
        };
        let mut stream = match stream_result {
            Ok(stream) => {
                if !initial_connect {
                    eprintln!(
                        "{}",
                        i18n::t("ai.agent_sdk.ambient.message.watch.reconnected")
                            .replace("{run_id}", &run_id)
                            .replace("{sequence}", &last_seen_sequence.to_string())
                    );
                }
                initial_connect = false;
                failures = 0;
                stream
            }
            Err(err) => {
                if initial_connect {
                    return Err(
                        err.context(i18n::t("ai.agent_sdk.ambient.message.watch.open_failed"))
                    );
                }

                failures += 1;
                let backoff = stream_retry_backoff(failures);
                eprintln!(
                    "{}",
                    i18n::t("ai.agent_sdk.ambient.message.watch.reconnect_failed")
                        .replace("{error}", &format!("{err:#}"))
                        .replace("{seconds}", &backoff.as_secs().to_string())
                );
                Timer::after(backoff).await;
                continue;
            }
        };

        loop {
            match stream.next().await {
                Some(Ok(reqwest_eventsource::Event::Open)) => {}
                Some(Ok(reqwest_eventsource::Event::Message(message))) => {
                    let event = match serde_json::from_str::<AgentRunEvent>(&message.data) {
                        Ok(event) => event,
                        Err(err) => {
                            eprintln!(
                                "{}",
                                i18n::t(
                                    "ai.agent_sdk.ambient.message.watch.malformed_event_payload"
                                )
                                .replace("{error}", &err.to_string())
                            );
                            continue;
                        }
                    };

                    if event.sequence <= last_seen_sequence {
                        continue;
                    }

                    if event.event_type != "new_message" || event.run_id != run_id {
                        last_seen_sequence = event.sequence;
                        continue;
                    }

                    let Some(message_id) = event.ref_id.clone() else {
                        eprintln!(
                            "{}",
                            i18n::t("ai.agent_sdk.ambient.message.watch.missing_ref_id")
                                .replace("{sequence}", &event.sequence.to_string())
                        );
                        last_seen_sequence = event.sequence;
                        continue;
                    };
                    let message_result = match scoped_task_id.as_ref() {
                        Some(task_id) => {
                            server_api
                                .read_agent_message_for_task(task_id, &message_id)
                                .await
                        }
                        None => ai_client.read_agent_message(&message_id).await,
                    };
                    let message = match message_result {
                        Ok(message) => message,
                        Err(err) => {
                            failures += 1;
                            let backoff = stream_retry_backoff(failures);
                            eprintln!(
                                "{}",
                                i18n::t("ai.agent_sdk.ambient.message.watch.hydrate_failed")
                                    .replace("{message_id}", &message_id)
                                    .replace("{error}", &format!("{err:#}"))
                                    .replace("{seconds}", &backoff.as_secs().to_string())
                            );
                            Timer::after(backoff).await;
                            break;
                        }
                    };

                    let record = MessageWatchEvent {
                        sequence: event.sequence,
                        message_id: message.message_id,
                        sender_run_id: message.sender_run_id,
                        subject: message.subject,
                        body: message.body,
                        occurred_at: event.occurred_at,
                    };
                    write_stream_record(&record)?;
                    last_seen_sequence = event.sequence;
                }
                Some(Err(err)) => {
                    failures += 1;
                    let backoff = stream_retry_backoff(failures);
                    eprintln!(
                        "{}",
                        i18n::t("ai.agent_sdk.ambient.message.watch.disconnected")
                            .replace("{error}", &err.to_string())
                            .replace("{seconds}", &backoff.as_secs().to_string())
                    );
                    Timer::after(backoff).await;
                    break;
                }
                None => {
                    failures += 1;
                    let backoff = stream_retry_backoff(failures);
                    eprintln!(
                        "{}",
                        i18n::t("ai.agent_sdk.ambient.message.watch.closed")
                            .replace("{seconds}", &backoff.as_secs().to_string())
                    );
                    Timer::after(backoff).await;
                    break;
                }
            }
        }
    }
}

fn print_send_message_response(
    response: &SendAgentMessageResponse,
    output_format: OutputFormat,
) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout();
    write_send_message_response(response, output_format, &mut stdout)
}

fn write_send_message_response<W>(
    response: &SendAgentMessageResponse,
    output_format: OutputFormat,
    mut output: W,
) -> anyhow::Result<()>
where
    W: std::io::Write,
{
    match output_format {
        OutputFormat::Json => super::output::write_json(response, &mut output),
        OutputFormat::Ndjson => super::output::write_json_line(response, &mut output),
        OutputFormat::Pretty | OutputFormat::Text => {
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.sent_count")
                    .replace("{count}", &response.message_ids.len().to_string())
            )?;
            if !response.message_ids.is_empty() {
                writeln!(
                    &mut output,
                    "{}",
                    i18n::t("ai.agent_sdk.ambient.message.ids")
                )?;
                for message_id in &response.message_ids {
                    writeln!(&mut output, "- {message_id}")?;
                }
            }
            Ok(())
        }
    }
}

fn print_read_message_response(
    response: &ReadAgentMessageResponse,
    output_format: OutputFormat,
) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout();
    write_read_message_response(response, output_format, &mut stdout)
}

fn write_read_message_response<W>(
    response: &ReadAgentMessageResponse,
    output_format: OutputFormat,
    mut output: W,
) -> anyhow::Result<()>
where
    W: std::io::Write,
{
    match output_format {
        OutputFormat::Json => super::output::write_json(response, &mut output),
        OutputFormat::Ndjson => super::output::write_json_line(response, &mut output),
        OutputFormat::Pretty | OutputFormat::Text => {
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.message_id")
                    .replace("{id}", &response.message_id)
            )?;
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.from")
                    .replace("{from}", &response.sender_run_id)
            )?;
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.subject")
                    .replace("{subject}", &response.subject)
            )?;
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.sent_at")
                    .replace("{sent_at}", &response.sent_at)
            )?;
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.delivered_at").replace(
                    "{delivered_at}",
                    format_optional_timestamp(response.delivered_at.as_deref())
                )
            )?;
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.read_at").replace(
                    "{read_at}",
                    format_optional_timestamp(response.read_at.as_deref())
                )
            )?;
            writeln!(&mut output)?;
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.body")
            )?;
            writeln!(&mut output, "{}", response.body)?;
            Ok(())
        }
    }
}

fn print_mark_message_delivered_result(
    message_id: &str,
    output_format: OutputFormat,
) -> anyhow::Result<()> {
    let mut stdout = std::io::stdout();
    write_mark_message_delivered_result(message_id, output_format, &mut stdout)
}

fn write_mark_message_delivered_result<W>(
    message_id: &str,
    output_format: OutputFormat,
    mut output: W,
) -> anyhow::Result<()>
where
    W: std::io::Write,
{
    let result = MessageDeliveredResult {
        message_id,
        delivered: true,
    };

    match output_format {
        OutputFormat::Json => super::output::write_json(&result, &mut output),
        OutputFormat::Ndjson => super::output::write_json_line(&result, &mut output),
        OutputFormat::Pretty | OutputFormat::Text => {
            writeln!(
                &mut output,
                "{}",
                i18n::t("ai.agent_sdk.ambient.message.marked_delivered")
                    .replace("{id}", message_id)
            )?;
            Ok(())
        }
    }
}

impl super::output::TableFormat for AgentMessageHeader {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new(i18n::t("ai.agent_sdk.ambient.message.table.message_id")),
            Cell::new(i18n::t("ai.agent_sdk.ambient.message.table.from")),
            Cell::new(i18n::t("ai.agent_sdk.ambient.message.table.subject")),
            Cell::new(i18n::t("ai.agent_sdk.ambient.message.table.sent_at")),
            Cell::new(i18n::t("ai.agent_sdk.ambient.message.table.delivered_at")),
            Cell::new(i18n::t("ai.agent_sdk.ambient.message.table.read_at")),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.message_id),
            Cell::new(&self.sender_run_id),
            Cell::new(&self.subject),
            Cell::new(&self.sent_at),
            Cell::new(format_optional_timestamp(self.delivered_at.as_deref())),
            Cell::new(format_optional_timestamp(self.read_at.as_deref())),
        ]
    }
}

/// Get a conversation by conversation ID.
pub fn get_conversation(ctx: &mut AppContext, conversation_id: String) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AmbientAgentRunner);
    runner.update(ctx, |runner, ctx| {
        runner.get_conversation(conversation_id, ctx)
    })
}

/// Get a conversation by run ID.
pub fn get_run_conversation(ctx: &mut AppContext, run_id: String) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AmbientAgentRunner);
    runner.update(ctx, |runner, ctx| runner.get_run_conversation(run_id, ctx))
}

impl AmbientAgentRunner {
    fn get_conversation(
        &self,
        conversation_id: String,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        let future = async move {
            let conversation = ai_client.get_public_conversation(&conversation_id).await?;
            let pretty = serde_json::to_string_pretty(&conversation)?;
            println!("{pretty}");
            Ok(())
        };
        self.spawn_command(future, ctx);

        Ok(())
    }

    fn get_run_conversation(
        &self,
        run_id: String,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        let future = async move {
            let conversation = ai_client.get_run_conversation(&run_id).await?;
            let pretty = serde_json::to_string_pretty(&conversation)?;
            println!("{pretty}");
            Ok(())
        };
        self.spawn_command(future, ctx);

        Ok(())
    }
}

impl warpui::Entity for AmbientAgentRunner {
    type Event = ();
}

impl SingletonEntity for AmbientAgentRunner {}

#[cfg(test)]
#[path = "ambient_tests.rs"]
mod tests;
