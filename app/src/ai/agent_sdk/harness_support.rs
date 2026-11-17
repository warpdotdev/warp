//! `warp harness-support` CLI dispatch and the singleton model all subcommands run async work on.
//!
//! Subcommands:
//! - [`ping`] — fetches the current run by task ID and prints its info.
//! - [`report_artifact`] — reports an artifact (a PR, a screenshot, or a video recording)
//!   back to the Oz platform.
//! - [`pr_media_markdown`] — prints the links and managed media blocks for the run's pull
//!   request description.

use std::path::Path;

use anyhow::{Context as _, Result};
#[cfg(target_os = "linux")]
use command::blocking::Command;
use warp_cli::GlobalOptions;
use warp_cli::agent::OutputFormat;
use warp_cli::harness_support::{
    FinishTaskArgs, HarnessSupportArgs, HarnessSupportCommand, NotifyUserArgs, ReportArtifactArgs,
    ReportArtifactCommand, ReportExternalReferenceArgs, ReportShutdownArgs, ScreenshotArtifactArgs,
    TaskStatus, VideoArtifactArgs,
};
use warp_core::features::FeatureFlag;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use super::artifact_upload::{
    FileArtifactUploadRequest, FileArtifactUploader, file_size_and_prefix_for_path,
};
use super::common::set_ambient_task_context_from_run_id;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::artifacts::Artifact;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::harness_support::{
    HarnessSupportClient, PullRequestMediaMarkdown, ReportArtifactResponse, UploadTarget,
};
use crate::server::server_api::presigned_upload::{FileUploadBody, upload_to_target};
use crate::util::image::{MIME_SNIFF_BYTES, infer_mime_type};

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
        HarnessSupportCommand::ReportArtifact(report_args) => report_artifact(
            ctx,
            runner,
            task_id,
            report_args,
            global_options.output_format,
        ),
        HarnessSupportCommand::PrMediaMarkdown => {
            pr_media_markdown(ctx, runner, global_options.output_format)
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
///
/// Pull requests are reported directly. Screenshots and video recordings are uploaded first:
/// a screenshot goes to a presigned target and becomes a run-scoped SCREENSHOT artifact, like
/// a native `report_screenshot` capture, so pull request descriptions embed it inline; a video
/// becomes a FILE artifact with a thumbnail, like a native `stop_recording` upload.
fn report_artifact(
    ctx: &mut AppContext,
    runner: ModelHandle<HarnessSupportRunner>,
    task_id: AmbientAgentTaskId,
    args: ReportArtifactArgs,
    output_format: OutputFormat,
) -> Result<()> {
    runner.update(ctx, |_, ctx| {
        let provider = ServerApiProvider::as_ref(ctx);
        let client = provider.get_harness_support_client();
        let server_api = provider.get();
        let ai_client = provider.get_ai_client();

        ctx.spawn(
            async move {
                match args.command {
                    ReportArtifactCommand::PullRequest(pr_args) => {
                        let artifact = Artifact::PullRequest {
                            url: pr_args.url,
                            branch: pr_args.branch,
                            repo: None,
                            number: None,
                        };
                        client.report_artifact(&artifact).await
                    }
                    ReportArtifactCommand::Screenshot(screenshot_args) => {
                        report_screenshot(client.as_ref(), screenshot_args).await
                    }
                    ReportArtifactCommand::Video(video_args) => {
                        let uploader = FileArtifactUploader::new(ai_client, server_api);
                        report_video(&uploader, task_id, video_args).await
                    }
                }
            },
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

/// Upload a screenshot to a presigned target and report it as a SCREENSHOT artifact.
async fn report_screenshot(
    client: &dyn HarnessSupportClient,
    args: ScreenshotArtifactArgs,
) -> Result<ReportArtifactResponse> {
    let (size_bytes, sniff_bytes) = file_size_and_prefix_for_path(&args.path, MIME_SNIFF_BYTES)?;
    if size_bytes == 0 {
        anyhow::bail!("Screenshot '{}' is empty", args.path.display());
    }
    let mime_type = infer_mime_type(&args.path, &sniff_bytes);
    let target = client
        .get_screenshot_upload_target(&mime_type, size_bytes)
        .await
        .context("Failed to get a screenshot upload target")?;
    let upload_target = with_content_type(&target.upload_target, &mime_type);
    upload_to_target(
        client.external_http_client(),
        &upload_target,
        FileUploadBody::new(args.path.clone()),
    )
    .await
    .with_context(|| format!("Failed to upload screenshot '{}'", args.path.display()))?;
    client
        .report_artifact(&Artifact::Screenshot {
            artifact_uid: target.artifact_uid,
            mime_type,
            description: args.description,
        })
        .await
}

/// Upload a video recording as a FILE artifact scoped to the run, then a best-effort
/// `{artifact_uid}-thumb.png` thumbnail the server links to it by filename, mirroring the native
/// recording upload so pull request descriptions render the video as a clickable thumbnail.
async fn report_video(
    uploader: &FileArtifactUploader,
    task_id: AmbientAgentTaskId,
    args: VideoArtifactArgs,
) -> Result<ReportArtifactResponse> {
    let request = FileArtifactUploadRequest {
        path: args.path.clone(),
        run_id: Some(task_id.clone()),
        conversation_id: None,
        title: args.title,
        description: args.description,
    };
    let association = uploader.resolve_upload_association(&request).await?;
    let uploaded = uploader
        .upload_with_association(request, association.clone())
        .await
        .with_context(|| format!("Failed to upload video '{}'", args.path.display()))?;
    let artifact_uid = uploaded.artifact.artifact_uid;

    if let Err(err) = upload_video_thumbnail(uploader, task_id, &args.path, &artifact_uid).await {
        log::warn!("Video {artifact_uid} was reported without a thumbnail: {err:#}");
    }

    Ok(ReportArtifactResponse { artifact_uid })
}

async fn upload_video_thumbnail(
    uploader: &FileArtifactUploader,
    task_id: AmbientAgentTaskId,
    video_path: &Path,
    video_artifact_uid: &str,
) -> Result<()> {
    let thumbnail_path =
        computer_use::generate_video_thumbnail(video_path, video_artifact_uid).await?;
    let upload_result = async {
        let request = FileArtifactUploadRequest {
            path: thumbnail_path.clone(),
            run_id: Some(task_id),
            conversation_id: None,
            title: None,
            description: None,
        };
        let association = uploader.resolve_upload_association(&request).await?;
        uploader.upload_with_association(request, association).await
    }
    .await;
    // The thumbnail file is ephemeral regardless of upload outcome.
    let _ = std::fs::remove_file(&thumbnail_path);
    upload_result?;
    Ok(())
}

/// Clone `target` with a `Content-Type` header matching `mime_type` unless the server already
/// set one, so the stored object carries the type the artifact is reported with.
fn with_content_type(target: &UploadTarget, mime_type: &str) -> UploadTarget {
    let mut headers = target.headers.clone();
    if !headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("content-type"))
    {
        headers.insert("Content-Type".to_string(), mime_type.to_string());
    }
    UploadTarget {
        url: target.url.clone(),
        method: target.method.clone(),
        headers,
        fields: target.fields.clone(),
    }
}

/// Print the links and managed media blocks the pull request description should carry.
fn pr_media_markdown(
    ctx: &mut AppContext,
    runner: ModelHandle<HarnessSupportRunner>,
    output_format: OutputFormat,
) -> Result<()> {
    runner.update(ctx, |_, ctx| {
        let client = ServerApiProvider::as_ref(ctx).get_harness_support_client();

        ctx.spawn(
            async move { client.pull_request_media_markdown().await },
            move |_, result, ctx| match result {
                Ok(media) => {
                    match output_format {
                        OutputFormat::Json | OutputFormat::Ndjson => {
                            let json = serde_json::to_string(&media).unwrap_or_else(|e| {
                                serde_json::json!({"error": e.to_string()}).to_string()
                            });
                            println!("{json}");
                        }
                        OutputFormat::Pretty | OutputFormat::Text => {
                            print!("{}", render_pr_media_markdown(&media));
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

/// Lay out the media blocks for pasting into a pull request body: videos above screenshots,
/// then regression context, plan links, and the session links.
fn render_pr_media_markdown(media: &PullRequestMediaMarkdown) -> String {
    let mut out = String::new();
    for block in [
        &media.video_markdown,
        &media.screenshot_markdown,
        &media.regressions_markdown,
    ] {
        if !block.trim().is_empty() {
            out.push_str(block.trim_end());
            out.push_str("\n\n");
        }
    }
    for plan in &media.plan_artifacts {
        out.push_str(&format!("- Plan: [{}]({})\n", plan.title, plan.link));
    }
    if !media.plan_artifacts.is_empty() {
        out.push('\n');
    }
    out.push_str(&format!("Conversation: {}\n", media.conversation_link));
    if !media.run_link.is_empty() {
        out.push_str(&format!("Run: {}\n", media.run_link));
    }
    out
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
    if let Some(evidence) = detect_oom_shutdown(args.exit_code, args.pid) {
        apply_oom_classification(&mut args, evidence);
    }
    runner.update(ctx, |_, ctx| {
        let client = ServerApiProvider::as_ref(ctx).get_harness_support_client();

        ctx.spawn(
            async move {
                match (args.error_category, args.error_message, args.exit_code) {
                    (Some(category), Some(message), exit_code) => {
                        client
                            .report_error_shutdown(category, message, exit_code)
                            .await
                    }
                    (None, None, None) => client.report_clean_shutdown().await,
                    _ => anyhow::bail!(
                        "--error-category and --error-message must be provided together; --exit-code is only valid for abnormal shutdown"
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

/// Results of attempting to infer whether or not the Warp agent process was OOM-killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OomDetectionResult {
    /// The process exited with status 137, conventionally reserved for the OOM killer.
    exit_status: bool,
    /// Kernel logs include a reference to the OOM killer terminating the process.
    kernel_logs: bool,
}

impl OomDetectionResult {
    fn was_oom(&self) -> bool {
        self.exit_status || self.kernel_logs
    }
}

fn apply_oom_classification(args: &mut ReportShutdownArgs, result: OomDetectionResult) {
    if !result.was_oom() {
        return;
    }

    log::info!(
        "Classifying shutdown as OOM: exit status 137 = {}, kernel logs = {}",
        result.exit_status,
        result.kernel_logs
    );
    tracing::info!(
        exit_status_137 = result.exit_status,
        kernel_logs = result.kernel_logs,
        "Classifying shutdown as OOM"
    );

    args.error_category = Some("oom".to_string());
    args.error_message = Some("The agent sandbox ran out of memory.".to_string());
}

fn detect_oom_shutdown(exit_code: Option<u8>, pid: Option<u32>) -> Option<OomDetectionResult> {
    let mut result = OomDetectionResult {
        exit_status: exit_code.is_some_and(|code| code == 137),
        kernel_logs: false,
    };

    // Only check kernel logs (potentially expensive) if the process failed.
    if exit_code.is_some_and(|code| code != 0) {
        result.kernel_logs = pid.is_some_and(kernel_logs_contain_oom_for_pid);
    }

    if result.was_oom() { Some(result) } else { None }
}

#[cfg(target_os = "linux")]
fn kernel_log_commands() -> [(&'static str, &'static [&'static str]); 2] {
    const DMESG_ARGS: &[&str] = &["--level=info,warn,err,crit,alert,emerg", "--color=never"];
    const JOURNALCTL_ARGS: &[&str] = &[
        "-k",
        "--priority=0..6",
        "--no-pager",
        "--grep=(?i)(oom-kill:|out of memory: killed process)",
    ];

    [("dmesg", DMESG_ARGS), ("journalctl", JOURNALCTL_ARGS)]
}

#[cfg(target_os = "linux")]
fn kernel_logs_contain_oom_for_pid(pid: u32) -> bool {
    kernel_log_commands()
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

#[cfg(any(target_os = "linux", test))]
fn oom_kill_line_matches_pid(line: &str, pid: u32) -> bool {
    let pid = pid.to_string();
    // Linux emits this victim format in `oom_kill_process`:
    // https://github.com/torvalds/linux/blob/fc5def2c2ad049588c875d86c7408537300ee43e/mm/oom_kill.c#L949-L950
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

    // Linux emits this victim format in `dump_oom_victim`:
    // https://github.com/torvalds/linux/blob/fc5def2c2ad049588c875d86c7408537300ee43e/mm/oom_kill.c#L442-L451
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
