//! `warp harness-support` CLI dispatch and the singleton model all subcommands run async work on.
//!
//! Subcommands:
//! - [`ping`] — fetches the current run by task ID and prints its info.
//! - [`report_artifact`] — reports an artifact (e.g. a PR) back to the Oz platform.

use anyhow::Result;
#[cfg(target_os = "linux")]
use command::blocking::Command;
use warp_cli::GlobalOptions;
use warp_cli::agent::OutputFormat;
use warp_cli::harness_support::{
    FinishTaskArgs, HarnessSupportArgs, HarnessSupportCommand, NotifyUserArgs, ReportArtifactArgs,
    ReportArtifactCommand, ReportExternalReferenceArgs, ReportShutdownArgs, TaskStatus,
};
use warp_core::features::FeatureFlag;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelHandle, SingletonEntity};

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
        HarnessSupportCommand::ReportExternalReference(args) => {
            report_external_reference(ctx, runner, args, global_options.output_format)
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

/// Report a URL-addressable external reference back to the Oz platform.
///
/// Validates that `--metadata` (when provided) parses as a JSON object before any
/// network call, then builds an `Artifact::ExternalReference` and reports it via
/// the same `report-artifact` endpoint used by pull requests.
fn report_external_reference(
    ctx: &mut AppContext,
    runner: ModelHandle<HarnessSupportRunner>,
    args: ReportExternalReferenceArgs,
    output_format: OutputFormat,
) -> Result<()> {
    let metadata = match args.metadata {
        Some(raw) => {
            let parsed: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|_| anyhow::anyhow!("--metadata must be a JSON object"))?;
            if !parsed.is_object() {
                anyhow::bail!("--metadata must be a JSON object");
            }
            Some(parsed)
        }
        None => None,
    };

    let artifact = Artifact::ExternalReference {
        reference_type: args.reference_type,
        url: args.url,
        title: args.title,
        metadata,
    };

    runner.update(ctx, |_, ctx| {
        let client = ServerApiProvider::as_ref(ctx).get_harness_support_client();

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
    mut args: ReportShutdownArgs,
    output_format: OutputFormat,
) -> Result<()> {
    let error_pair_is_valid = matches!(
        (&args.error_category, &args.error_message),
        (Some(_), Some(_)) | (None, None)
    );
    if error_pair_is_valid && let Some(message) = detect_oom_shutdown(args.exit_code, args.pid) {
        args.error_category = Some("oom".to_string());
        args.error_message = Some(message);
    }
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

fn detect_oom_shutdown(exit_code: Option<u8>, pid: Option<u32>) -> Option<String> {
    let exit_code = exit_code.filter(|exit_code| *exit_code != 0)?;
    let kernel_evidence = pid.is_some_and(kernel_logs_contain_oom_for_pid);
    oom_shutdown_message(exit_code == 137, kernel_evidence)
}

fn oom_shutdown_message(exit_137: bool, kernel_evidence: bool) -> Option<String> {
    match (exit_137, kernel_evidence) {
        (true, true) => {
            Some("agent process was OOM-killed (exit status 137 and kernel evidence)".to_string())
        }
        (true, false) => Some("agent process was OOM-killed (exit status 137)".to_string()),
        (false, true) => Some("agent process was OOM-killed (kernel evidence)".to_string()),
        (false, false) => None,
    }
}

#[cfg(target_os = "linux")]
fn kernel_logs_contain_oom_for_pid(pid: u32) -> bool {
    [
        ("dmesg", &[][..]),
        ("journalctl", &["-k", "--no-pager"][..]),
    ]
    .into_iter()
    .filter_map(|(program, args)| Command::new(program).args(args).output().ok())
    .filter(|output| output.status.success())
    .any(|output| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| oom_kill_line_matches_pid(line, pid))
    })
}

#[cfg(not(target_os = "linux"))]
fn kernel_logs_contain_oom_for_pid(_: u32) -> bool {
    false
}

fn oom_kill_line_matches_pid(line: &str, pid: u32) -> bool {
    let pid = pid.to_string();
    let killed_process = (line.contains("Out of memory:") || line.contains("out of memory:"))
        && line
            .match_indices("Killed process ")
            .any(|(index, prefix)| {
                let suffix = &line[index + prefix.len()..];
                suffix
                    .strip_prefix(&pid)
                    .is_some_and(|suffix| suffix.starts_with(" ("))
            });
    if killed_process {
        return true;
    }

    line.contains("oom-kill:")
        && line.match_indices("pid=").any(|(index, prefix)| {
            let field_start = index + prefix.len();
            let valid_start = index == 0
                || line[..index].chars().next_back().is_some_and(|character| {
                    character == ',' || character == ':' || character.is_ascii_whitespace()
                });
            valid_start
                && line[field_start..]
                    .strip_prefix(&pid)
                    .is_some_and(|suffix| {
                        suffix.chars().next().is_none_or(|character| {
                            character == ',' || character.is_ascii_whitespace()
                        })
                    })
        })
}

/// Singleton model for running async harness-support operations.
struct HarnessSupportRunner;

impl warpui::Entity for HarnessSupportRunner {
    type Event = ();
}

impl SingletonEntity for HarnessSupportRunner {}

#[cfg(test)]
#[path = "harness_support_tests.rs"]
mod tests;
