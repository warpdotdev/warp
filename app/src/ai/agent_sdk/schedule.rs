use chrono::{DateTime, Utc};
use comfy_table::Cell;
use futures::future;
use serde::Serialize;
use warp_cli::agent::OutputFormat;
use warp_cli::schedule::{
    CreateScheduleArgs, DeleteScheduleArgs, GetScheduleArgs, PauseScheduleArgs, ScheduleCommand,
    ScheduleSubcommand, UnpauseScheduleArgs, UpdateScheduleArgs,
};
use warp_cli::GlobalOptions;
use warp_graphql::queries::get_scheduled_agent_history::ScheduledAgentHistory;
use warpui::platform::TerminationMode;
use warpui::{AppContext, SingletonEntity};

use super::common::{EnvironmentChoice, ResolveConfigurationError};
use super::output::{self, TableFormat};
use crate::ai::ambient_agents::scheduled::{
    CloudScheduledAmbientAgent, ScheduledAgentManager, ScheduledAmbientAgent, UpdateScheduleParams,
};
use crate::ai::ambient_agents::AgentConfigSnapshot;
use crate::cloud_object::{CloudObject, CloudObjectLookup as _};
use crate::server::ids::{ServerId, SyncId};
use crate::util::time_format::format_approx_duration_from_now_utc;

/// Run a scheduled agent command.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: ScheduleCommand,
) -> anyhow::Result<()> {
    let output_format = global_options.output_format;
    match command.into_subcommand() {
        ScheduleSubcommand::Create(args) => create(ctx, args),
        ScheduleSubcommand::List => list(ctx, output_format),
        ScheduleSubcommand::Get(args) => get(ctx, output_format, args),
        ScheduleSubcommand::Pause(args) => pause(ctx, args),
        ScheduleSubcommand::Unpause(args) => unpause(ctx, args),
        ScheduleSubcommand::Update(args) => update(ctx, args),
        ScheduleSubcommand::Delete(args) => delete(ctx, args),
    }
}

fn create(ctx: &mut AppContext, args: CreateScheduleArgs) -> anyhow::Result<()> {
    ScheduledAgentManager::handle(ctx).update(ctx, move |_manager, ctx| {
        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        let setup_future = future::try_join(refresh_future, warp_drive_sync_future);

        ctx.spawn(setup_future, move |manager, setup_result, ctx| {
            if let Err(err) = setup_result {
                super::report_fatal_error(err, ctx);
                return;
            }

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
                    eprintln!("Scheduling agent to run without an environment.");
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

            let owner =
                match super::common::resolve_owner(args.scope.team, args.scope.personal, ctx) {
                    Ok(owner) => owner,
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                };

            let cli_mcp_servers =
                match super::mcp_config::build_mcp_servers_from_specs(&args.mcp_specs) {
                    Ok(mcp_servers) => mcp_servers,
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                };

            let merged_config = super::config_file::merge_with_precedence(
                loaded_file.as_ref(),
                crate::ai::ambient_agents::AgentConfigSnapshot {
                    name: None,
                    environment_id,
                    // TODO(REMOTE-1936): support --runner for scheduled agents.
                    runner_id: None,
                    model_id: args.model.model.clone(),
                    base_prompt: None,
                    mcp_servers: cli_mcp_servers,
                    profile_id: None,
                    worker_host: args.worker_host,
                    skill_spec: args.skill.map(|s| s.to_string()),
                    // TODO(QUALITY-294): Support computer use flag in scheduled agents.
                    computer_use_enabled: None,
                    // TODO(REMOTE-1134): Support harness flag for scheduled agents.
                    harness: None,
                    harness_auth_secrets: None,
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
                    super::report_fatal_error(anyhow::anyhow!(err), ctx);
                    return;
                }
            };

            let mut agent_config = merged_config;
            agent_config.model_id = model_id;

            let prompt = args.prompt.unwrap_or_default();
            let mut config = ScheduledAmbientAgent::new(args.name, args.cron, true, prompt);
            config.agent_config = agent_config;

            // Print something here because scheduling an agent can take a while.
            println!("Scheduling agent {}...", config.name);
            let create_future = manager.create_schedule(config, owner, ctx);
            ctx.spawn(create_future, |_manager, result, ctx| match result {
                Ok(sync_id) => {
                    println!("Scheduled agent: {sync_id}");
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            });
        });
    });

    Ok(())
}

#[derive(Serialize)]
struct ScheduleInfo {
    id: String,
    name: String,
    cron_schedule: String,
    paused: bool,
    last_ran: Option<DateTime<Utc>>,
    next_run: Option<DateTime<Utc>>,
    scope: String,
    prompt: String,
    last_spawn_error: Option<String>,
    agent_config: AgentConfigSnapshot,
}

impl ScheduleInfo {
    fn new(
        id: String,
        scope: String,
        config: ScheduledAmbientAgent,
        history: Option<&ScheduledAgentHistory>,
    ) -> Self {
        let last_ran = history.and_then(|h| h.last_ran.as_ref().map(|t| t.utc()));
        let next_run = history.and_then(|h| h.next_run.as_ref().map(|t| t.utc()));
        ScheduleInfo {
            id,
            name: config.name,
            cron_schedule: config.cron_schedule,
            paused: !config.enabled,
            last_ran,
            next_run,
            scope,
            prompt: config.prompt,
            last_spawn_error: config.last_spawn_error,
            agent_config: config.agent_config,
        }
    }

    fn last_ran_display(&self) -> String {
        let timestamp = self
            .last_ran
            .map(format_approx_duration_from_now_utc)
            .unwrap_or("-".to_string());

        if self.last_spawn_error.is_some() {
            format!("❌ {}", timestamp)
        } else {
            timestamp
        }
    }

    fn next_run_display(&self) -> String {
        // We can't use format_approx_duration_from_now_utc because the date is in the future.
        // We use RTC 2822 rather than RFC 3339 because it's more human-readable.
        self.next_run
            .map(|dt| dt.to_rfc2822())
            .unwrap_or("-".to_string())
    }
}

impl TableFormat for ScheduleInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("ID"),
            Cell::new("Name"),
            Cell::new("Schedule"),
            Cell::new("Paused"),
            Cell::new("Last ran"),
            Cell::new("Next run"),
            Cell::new("Scope"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        let paused_display = if self.paused { "Yes" } else { "No" };
        vec![
            Cell::new(&self.id),
            Cell::new(&self.name),
            Cell::new(&self.cron_schedule),
            Cell::new(paused_display),
            Cell::new(self.last_ran_display()),
            Cell::new(self.next_run_display()),
            Cell::new(&self.scope),
        ]
    }
}

fn print_schedule_info(info: &ScheduleInfo, output_format: OutputFormat) -> anyhow::Result<()> {
    let paused_display = if info.paused { "Yes" } else { "No" };

    match output_format {
        OutputFormat::Json => {
            serde_json::to_writer(std::io::stdout(), info)?;
            Ok(())
        }
        OutputFormat::Ndjson => output::write_json_line(info, std::io::stdout()),
        OutputFormat::Text => {
            println!("Name: {}", info.name);
            println!("Cron schedule: {}", info.cron_schedule);
            println!("Paused: {paused_display}");

            let last_ran = info.last_ran_display();
            let next_run = info.next_run_display();
            println!("Last ran: {last_ran}");
            if let Some(error) = &info.last_spawn_error {
                println!("Last error: {error}");
            }
            println!("Next run: {next_run}");

            println!("Prompt: {}", info.prompt);

            if let Some(environment_id) = &info.agent_config.environment_id {
                println!("Environment ID: {environment_id}");
            }
            if let Some(model_id) = &info.agent_config.model_id {
                println!("Model ID: {model_id}");
            }
            if let Some(agent_name) = &info.agent_config.name {
                println!("Agent name: {agent_name}");
            }
            if let Some(skill_spec) = &info.agent_config.skill_spec {
                println!("Skill: {skill_spec}");
            }
            if let Some(worker_host) = &info.agent_config.worker_host {
                println!("Host: {worker_host}");
            }

            Ok(())
        }
        OutputFormat::Pretty => {
            let mut table = output::standard_table();
            table.add_row(vec![Cell::new("Name"), Cell::new(&info.name)]);
            table.add_row(vec![
                Cell::new("Cron schedule"),
                Cell::new(&info.cron_schedule),
            ]);
            table.add_row(vec![Cell::new("Paused"), Cell::new(paused_display)]);

            let last_ran = info.last_ran_display();
            let next_run = info.next_run_display();
            table.add_row(vec![Cell::new("Last ran"), Cell::new(last_ran)]);
            if let Some(error) = &info.last_spawn_error {
                table.add_row(vec![Cell::new("Last error"), Cell::new(error)]);
            }
            table.add_row(vec![Cell::new("Next run"), Cell::new(next_run)]);

            table.add_row(vec![Cell::new("Prompt"), Cell::new(&info.prompt)]);

            if let Some(environment_id) = &info.agent_config.environment_id {
                table.add_row(vec![Cell::new("Environment ID"), Cell::new(environment_id)]);
            }
            if let Some(model_id) = &info.agent_config.model_id {
                table.add_row(vec![Cell::new("Model ID"), Cell::new(model_id)]);
            }
            if let Some(agent_name) = &info.agent_config.name {
                table.add_row(vec![Cell::new("Agent name"), Cell::new(agent_name)]);
            }
            if let Some(skill_spec) = &info.agent_config.skill_spec {
                table.add_row(vec![Cell::new("Skill"), Cell::new(skill_spec)]);
            }
            if let Some(worker_host) = &info.agent_config.worker_host {
                table.add_row(vec![Cell::new("Host"), Cell::new(worker_host)]);
            }

            println!("{table}");
            Ok(())
        }
    }
}

fn pause(ctx: &mut AppContext, args: PauseScheduleArgs) -> anyhow::Result<()> {
    let schedule_id = SyncId::ServerId(ServerId::try_from(args.schedule_id)?);

    ScheduledAgentManager::handle(ctx).update(ctx, move |_manager, ctx| {
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        ctx.spawn(warp_drive_sync_future, move |manager, result, ctx| {
            if let Err(err) = result {
                super::report_fatal_error(err, ctx);
                return;
            }

            println!("Pausing agent...");
            let pause_future = manager.pause_schedule(schedule_id, ctx);
            ctx.spawn(pause_future, |_manager, result, ctx| match result {
                Ok(()) => {
                    println!("Schedule paused");
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            });
        });
    });

    Ok(())
}

fn unpause(ctx: &mut AppContext, args: UnpauseScheduleArgs) -> anyhow::Result<()> {
    let schedule_id = SyncId::ServerId(ServerId::try_from(args.schedule_id)?);

    ScheduledAgentManager::handle(ctx).update(ctx, move |_manager, ctx| {
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        ctx.spawn(warp_drive_sync_future, move |manager, result, ctx| {
            if let Err(err) = result {
                super::report_fatal_error(err, ctx);
                return;
            }

            println!("Resuming agent...");
            let unpause_future = manager.unpause_schedule(schedule_id, ctx);
            ctx.spawn(unpause_future, |_manager, result, ctx| match result {
                Ok(()) => {
                    println!("Schedule unpaused");
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            });
        });
    });

    Ok(())
}

fn update(ctx: &mut AppContext, args: UpdateScheduleArgs) -> anyhow::Result<()> {
    let schedule_id = SyncId::ServerId(ServerId::try_from(args.schedule_id)?);

    ScheduledAgentManager::handle(ctx).update(ctx, move |_manager, ctx| {
        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        let setup_future = future::try_join(refresh_future, warp_drive_sync_future);

        ctx.spawn(setup_future, move |manager, setup_result, ctx| {
            if let Err(err) = setup_result {
                super::report_fatal_error(err, ctx);
                return;
            }

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

            let file_config = loaded_file.as_ref().map(|f| &f.file);

            // We must wait until after workspace metadata is refreshed to check available LLMs.
            let model_id = match args
                .model
                .model
                .as_deref()
                .or_else(|| file_config.and_then(|f| f.model_id.as_deref()))
                .map(|model_id| super::common::validate_agent_mode_base_model_id(model_id, ctx))
                .transpose()
            {
                Ok(id) => id.map(|id| id.to_string()),
                Err(err) => {
                    super::report_fatal_error(anyhow::anyhow!(err), ctx);
                    return;
                }
            };

            let mut environment_args = args.environment;
            if environment_args.environment.is_none() && !environment_args.remove_environment {
                if let Some(environment_id) = file_config.and_then(|f| f.environment_id.clone()) {
                    environment_args.environment = Some(environment_id);
                }
            }

            let environment_id = match EnvironmentChoice::resolve_for_update(environment_args, ctx)
            {
                Ok(choice) => choice.map(|c| match c {
                    EnvironmentChoice::None => None,
                    EnvironmentChoice::Environment { id, .. } => Some(id),
                }),
                Err(ResolveConfigurationError::Canceled) => {
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                    return;
                }
                Err(err) => {
                    super::report_fatal_error(anyhow::anyhow!(err), ctx);
                    return;
                }
            };

            // MCP update semantics are patch-only:
            // - file and CLI MCP servers are treated as upserts (CLI wins on key conflicts)
            // - `--remove-mcp` removes keys
            // If both are present, removals win by filtering removed names out of the upsert payload.
            let mut mcp_servers_upsert = match file_config.and_then(|f| f.mcp_servers.clone()) {
                Some(map) if map.is_empty() => None,
                Some(map) => Some(map),
                None => None,
            };

            let cli_mcp_servers_upsert =
                match super::mcp_config::build_mcp_servers_from_specs(&args.mcp_specs) {
                    Ok(mcp_servers) => mcp_servers,
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                };

            if let Some(cli_map) = cli_mcp_servers_upsert {
                let merged = mcp_servers_upsert.get_or_insert_with(serde_json::Map::new);
                for (name, config_value) in cli_map {
                    merged.insert(name, config_value);
                }
            }

            if let Some(map) = mcp_servers_upsert.as_mut() {
                for name in &args.remove_mcp {
                    map.remove(name);
                }
                if map.is_empty() {
                    mcp_servers_upsert = None;
                }
            }

            // Handle skill update semantics: --skill sets it, --remove-skill clears it
            let skill_spec = if args.remove_skill {
                Some(None)
            } else {
                args.skill.map(|s| Some(s.to_string()))
            };

            println!("Updating agent...");
            let update_future = manager.update_schedule(
                schedule_id,
                UpdateScheduleParams {
                    name: args.name,
                    cron: args.cron,
                    model_id,
                    environment_id,
                    base_prompt: file_config.and_then(|f| f.base_prompt.clone()),
                    prompt: args.prompt,
                    mcp_servers_upsert,
                    remove_mcp_server_names: args.remove_mcp,
                    skill_spec,
                    worker_host: args.worker_host,
                },
                ctx,
            );
            ctx.spawn(update_future, |_manager, result, ctx| match result {
                Ok(()) => {
                    println!("Schedule updated");
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            });
        });
    });

    Ok(())
}

/// List all scheduled agents available to the current user.
fn list(ctx: &mut AppContext, output_format: OutputFormat) -> anyhow::Result<()> {
    ScheduledAgentManager::handle(ctx).update(ctx, move |_manager, ctx| {
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        ctx.spawn(warp_drive_sync_future, move |manager, result, ctx| {
            if let Err(err) = result {
                super::report_fatal_error(err, ctx);
                return;
            }

            let mut schedules = manager.list_schedules(ctx);
            schedules.sort_by_key(|schedule| schedule.model().string_model.name.clone());

            let futures = schedules.into_iter().map(|schedule| {
                let config = schedule.model().string_model.clone();
                let sync_id = schedule.sync_id();
                let scope = super::common::format_owner(&schedule.permissions().owner).to_string();

                // TODO(ben): Consider a bulk lookup API for scheduled agent history.
                let history_future = manager.fetch_schedule_history(sync_id, ctx);

                async move {
                    // Try to fetch the scheduled agent history, but still show output if this fails.
                    let history = match history_future.await {
                        Ok(v) => v,
                        Err(err) => {
                            log::warn!("Failed to fetch scheduled agent history: {err:#}");
                            None
                        }
                    };

                    let id = match sync_id {
                        SyncId::ServerId(server_id) => server_id.to_string(),
                        SyncId::ClientId(_) => "Unsynced".to_string(),
                    };

                    ScheduleInfo::new(id, scope, config, history.as_ref())
                }
            });

            let output_format = output_format;
            ctx.spawn(
                futures::future::join_all(futures),
                move |_manager, infos, ctx| {
                    output::print_list(infos, output_format);

                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                },
            );
        });
    });

    Ok(())
}

fn get(
    ctx: &mut AppContext,
    output_format: OutputFormat,
    args: GetScheduleArgs,
) -> anyhow::Result<()> {
    let schedule_id = SyncId::ServerId(ServerId::try_from(args.schedule_id)?);

    ScheduledAgentManager::handle(ctx).update(ctx, move |_manager, ctx| {
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        ctx.spawn(warp_drive_sync_future, move |manager, result, ctx| {
            if let Err(err) = result {
                super::report_fatal_error(err, ctx);
                return;
            }

            let Some(schedule) = CloudScheduledAmbientAgent::get_by_id(&schedule_id, ctx) else {
                super::report_fatal_error(anyhow::anyhow!("Schedule not found"), ctx);
                return;
            };

            let id = match &schedule_id {
                SyncId::ServerId(server_id) => server_id.to_string(),
                SyncId::ClientId(_) => "Unsynced".to_string(),
            };
            let scope = super::common::format_owner(&schedule.permissions().owner).to_string();
            let config = schedule.model().string_model.clone();

            // Don't hold references into the CloudObject store across an async spawn.
            let history_future = manager.fetch_schedule_history(schedule_id, ctx);

            ctx.spawn(history_future, move |_manager, history, ctx| {
                let history = match history {
                    Ok(v) => v,
                    Err(err) => {
                        log::warn!("Failed to fetch scheduled agent history: {err:#}");
                        None
                    }
                };

                let info = ScheduleInfo::new(id, scope, config, history.as_ref());
                if let Err(err) = print_schedule_info(&info, output_format) {
                    super::report_fatal_error(err, ctx);
                    return;
                }

                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            });
        });
    });

    Ok(())
}

fn delete(ctx: &mut AppContext, args: DeleteScheduleArgs) -> anyhow::Result<()> {
    let schedule_id = SyncId::ServerId(ServerId::try_from(args.schedule_id)?);

    ScheduledAgentManager::handle(ctx).update(ctx, move |_manager, ctx| {
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        ctx.spawn(warp_drive_sync_future, move |manager, result, ctx| {
            if let Err(err) = result {
                super::report_fatal_error(err, ctx);
                return;
            }

            println!("Deleting agent...");
            let delete_future = manager.delete_schedule(schedule_id, ctx);
            ctx.spawn(delete_future, |_manager, result, ctx| match result {
                Ok(()) => {
                    println!("Schedule deleted");
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            });
        });
    });

    Ok(())
}
