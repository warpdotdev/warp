//! `warp harness-support` CLI dispatch and the singleton model all subcommands run async work on.
//!
//! Subcommands:
//! - [`ping`] — fetches the current run by task ID and prints its info.
//! - [`report_artifact`] — reports an artifact (e.g. a PR) back to the Oz platform.
use anyhow::Result;
use warp_cli::agent::OutputFormat;
use warp_cli::harness_support::{
    FinishTaskArgs, HarnessSupportArgs, HarnessSupportCommand, NotifyUserArgs, ReportArtifactArgs,
    ReportArtifactCommand, ReportShutdownArgs, TaskStatus,
};
use warp_cli::GlobalOptions;
use warp_core::features::FeatureFlag;
use warpui::{platform::TerminationMode, AppContext, ModelHandle, SingletonEntity};

use super::common::set_ambient_task_context_from_run_id;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::artifacts::Artifact;
use crate::server::server_api::ServerApiProvider;

/// Run harness-support commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    args: HarnessSupportArgs,
) -> Result<()> {
    if !FeatureFlag::AgentHarness.is_enabled() {
        return Err(anyhow::anyhow!("This feature is not enabled"));
    }

    // Store the run ID so that it's included on all server requests, along with a workload token.
    let task_id = set_ambient_task_context_from_run_id(ctx, &args.run_id)?;
    let runner = ctx.add_singleton_model(|_| HarnessSupportRunner);

    match args.command {
        HarnessSupportCommand::Ping => ping(ctx, runner, task_id, global_options.output_format),
        HarnessSupportCommand::ReportArtifact(report_args) => {
            report_artifact(ctx, runner, report_args, global_options.output_format)
        }
        HarnessSupportCommand::NotifyUser(notify_args) => {
            notify_user(ctx, runner, notify_args, global_options.output_format)
        }
        HarnessSupportCommand::FinishTask(finish_args) => {
            finish_task(ctx, runner, finish_args, global_options.output_format)
        }
        HarnessSupportCommand::ReportShutdown(shutdown_args) => {
            report_shutdown(ctx, runner, shutdown_args, global_options.output_format)
        }
    }
}

/// Fetch the current run by ID and print its info.
fn ping(
    ctx: &mut AppContext,
    runner: ModelHandle<HarnessSupportRunner>,
    task_id: AmbientAgentTaskId,
    output_format: OutputFormat,
) -> Result<()> {
    runner.update(ctx, |_, ctx| {
        let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client();

        ctx.spawn(
            async move {
                let task = ai_client.get_ambient_agent_task(&task_id).await?;
                Ok(task)
            },
            move |_, result, ctx| match result {
                Ok(task) => {
                    match output_format {
                        OutputFormat::Json | OutputFormat::Ndjson => {
                            let json = serde_json::to_string(&task).unwrap_or_else(|e| {
                                serde_json::json!({"error": e.to_string()}).to_string()
                            });
                            println!("{json}");
                        }
                        OutputFormat::Pretty | OutputFormat::Text => {
                            super::ambient::print_tasks(&[task]);
                        }
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            },
        );
    });

    Ok(())
}

/// Report an artifact back to the Oz platform.
fn report_artifact(
    ctx: &mut AppContext,
    runner: ModelHandle<HarnessSupportRunner>,
    args: ReportArtifactArgs,
    output_format: OutputFormat,
) -> Result<()> {
    runner.update(ctx, |_, ctx| {
        let client = ServerApiProvider::as_ref(ctx).get_harness_support_client();

        let artifact = match args.command {
            ReportArtifactCommand::PullRequest(pr_args) => Artifact::PullRequest {
                url: pr_args.url,
                branch: pr_args.branch,
                repo: None,
                number: None,
            },
        };

        ctx.spawn(
            async move { client.report_artifact(&artifact).await },
            move |_, result, ctx| match result {
                Ok(response) => {
                    match output_format {
                        OutputFormat::Json | OutputFormat::Ndjson => {
                            let json = serde_json::to_string(&response).unwrap_or_else(|e| {
                                serde_json::json!({"error": e.to_string()}).to_string()
                            });
                            println!("{json}");
                        }
                        OutputFormat::Pretty | OutputFormat::Text => {
                            println!("Artifact reported: {}", response.artifact_uid);
                        }
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            },
        );
    });

    Ok(())
}

/// Send a progress notification to the task's originating platform.
fn notify_user(
    ctx: &mut AppContext,
    runner: ModelHandle<HarnessSupportRunner>,
    args: NotifyUserArgs,
    output_format: OutputFormat,
) -> Result<()> {
    runner.update(ctx, |_, ctx| {
        let client = ServerApiProvider::as_ref(ctx).get_harness_support_client();

        ctx.spawn(
            async move { client.notify_user(&args.message).await },
            move |_, result, ctx| match result {
                Ok(()) => {
                    match output_format {
                        OutputFormat::Json | OutputFormat::Ndjson => {
                            println!("{{}}");
                        }
                        OutputFormat::Pretty | OutputFormat::Text => {
                            println!("Notification sent.");
                        }
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            },
        );
    });

    Ok(())
}

/// Report task completion or failure.
fn finish_task(
    ctx: &mut AppContext,
    runner: ModelHandle<HarnessSupportRunner>,
    args: FinishTaskArgs,
    output_format: OutputFormat,
) -> Result<()> {
    runner.update(ctx, |_, ctx| {
        let client = ServerApiProvider::as_ref(ctx).get_harness_support_client();

        ctx.spawn(
            async move {
                let success = args.status == TaskStatus::Success;
                client.finish_task(success, &args.summary).await
            },
            move |_, result, ctx| match result {
                Ok(()) => {
                    match output_format {
                        OutputFormat::Json | OutputFormat::Ndjson => {
                            println!("{{}}");
                        }
                        OutputFormat::Pretty | OutputFormat::Text => {
                            println!("Task finished.");
                        }
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            },
        );
    });

    Ok(())
}

/// Report that the agent process is shutting down.
///
/// Routes to `report_clean_shutdown` or `report_error_shutdown` on the API client
/// depending on whether error arguments were provided.
fn report_shutdown(
    ctx: &mut AppContext,
    runner: ModelHandle<HarnessSupportRunner>,
    args: ReportShutdownArgs,
    output_format: OutputFormat,
) -> Result<()> {
    runner.update(ctx, |_, ctx| {
        let client = ServerApiProvider::as_ref(ctx).get_harness_support_client();

        ctx.spawn(
            async move {
                match (args.error_category, args.error_message) {
                    (Some(category), Some(message)) => {
                        client.report_error_shutdown(category, message).await
                    }
                    (None, None) => client.report_clean_shutdown().await,
                    _ => anyhow::bail!(
                        "--error-category and --error-message must be provided together"
                    ),
                }
            },
            move |_, result, ctx| match result {
                Ok(()) => {
                    match output_format {
                        OutputFormat::Json | OutputFormat::Ndjson => {
                            println!("{{}}");
                        }
                        OutputFormat::Pretty | OutputFormat::Text => {
                            println!("Shutdown reported.");
                        }
                    }
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            },
        );
    });

    Ok(())
}

/// Singleton model for running async harness-support operations.
struct HarnessSupportRunner;

impl warpui::Entity for HarnessSupportRunner {
    type Event = ();
}

impl SingletonEntity for HarnessSupportRunner {}
