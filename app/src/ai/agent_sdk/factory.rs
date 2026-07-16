//! Commands to manage factory config sources via the public API.

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail};
use warp_cli::agent::OutputFormat;
use warp_cli::factory::{
    ApplyFactoryArgs, ExportFactoryArgs, FactoryCommand, InitFactoryArgs, LinkFactoryArgs,
    PlanFactoryArgs, StatusFactoryArgs,
};
use warp_cli::GlobalOptions;
use warpui::platform::TerminationMode;
use warpui::r#async::Timer;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::server::server_api::factory::{
    FactoryClient, FactoryExportResponse, FactoryPlannedChange, FactoryRepository,
    FactoryResourceError, FactoryResponse, FactorySource, FactorySourceRequest,
    FactorySyncDryRunResult, FactorySyncState, FactorySyncStatusResponse, FactorySyncSummary,
    GITHUB_CODE_FORGE,
};
use crate::server::server_api::ServerApiProvider;

/// How often `apply --wait` polls the sync status.
const WAIT_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Maximum number of `apply --wait` polls before giving up.
const WAIT_MAX_POLLS: usize = 150;

/// Singleton model that runs async work for factory CLI commands.
struct FactoryCommandRunner;

impl warpui::Entity for FactoryCommandRunner {
    type Event = ();
}

impl SingletonEntity for FactoryCommandRunner {}

/// Run a factory command.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: FactoryCommand,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| FactoryCommandRunner);
    let output_format = global_options.output_format;
    match command {
        FactoryCommand::Link(args) if args.unlink => runner.update(ctx, |runner, ctx| {
            runner.unlink(args.factory_uid, output_format, ctx)
        }),
        FactoryCommand::Link(args) => {
            runner.update(ctx, |runner, ctx| runner.link(args, output_format, ctx))
        }
        FactoryCommand::Unlink(args) => runner.update(ctx, |runner, ctx| {
            runner.unlink(args.factory_uid, output_format, ctx)
        }),
        FactoryCommand::Status(args) => {
            runner.update(ctx, |runner, ctx| runner.status(args, output_format, ctx))
        }
        FactoryCommand::Plan(args) => {
            runner.update(ctx, |runner, ctx| runner.plan(args, output_format, ctx))
        }
        FactoryCommand::Apply(args) => {
            runner.update(ctx, |runner, ctx| runner.apply(args, output_format, ctx))
        }
        FactoryCommand::Init(args) => {
            runner.update(ctx, |runner, ctx| runner.init(args, output_format, ctx))
        }
        FactoryCommand::Export(args) => {
            runner.update(ctx, |runner, ctx| runner.export(args, output_format, ctx))
        }
    }
}

impl FactoryCommandRunner {
    fn spawn_command(
        &self,
        future: impl warpui::r#async::Spawnable<Output = anyhow::Result<()>>,
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

    fn link(
        &self,
        args: LinkFactoryArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let factory_client = ServerApiProvider::as_ref(ctx).get_factory_client();
        let future = async move {
            let repo = args
                .repo
                .ok_or_else(|| anyhow!("--repo is required unless --unlink is set"))?;
            let request = FactorySourceRequest {
                code_forge: GITHUB_CODE_FORGE.to_string(),
                repository: FactoryRepository {
                    owner: repo.owner,
                    repo: repo.repo,
                },
                r#ref: args.branch,
                path: args.path,
            };
            let factory = factory_client
                .link_factory_source(&args.factory_uid, request)
                .await?;
            write_linked_factory(&factory, output_format, &mut std::io::stdout())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn unlink(
        &self,
        factory_uid: String,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let factory_client = ServerApiProvider::as_ref(ctx).get_factory_client();
        let future = async move {
            factory_client.unlink_factory_source(&factory_uid).await?;
            match output_format {
                OutputFormat::Json | OutputFormat::Ndjson => {
                    super::output::write_json_line(
                        &serde_json::json!({ "factory_uid": factory_uid, "unlinked": true }),
                        std::io::stdout(),
                    )?;
                }
                OutputFormat::Pretty | OutputFormat::Text => {
                    println!("Factory {factory_uid} unlinked; it is now live-managed.");
                }
            }
            Ok(())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn status(
        &self,
        args: StatusFactoryArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let factory_client = ServerApiProvider::as_ref(ctx).get_factory_client();
        let future = async move {
            let status = factory_client
                .get_factory_sync_status(&args.factory_uid)
                .await?;
            write_status(&status, output_format, &mut std::io::stdout())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn plan(
        &self,
        args: PlanFactoryArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let factory_client = ServerApiProvider::as_ref(ctx).get_factory_client();
        let future = async move {
            let result = factory_client
                .sync_factory_dry_run(&args.factory_uid, args.sha)
                .await?;
            write_plan(&result, output_format, &mut std::io::stdout())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn apply(
        &self,
        args: ApplyFactoryArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let factory_client = ServerApiProvider::as_ref(ctx).get_factory_client();
        let future = async move {
            let accepted = factory_client
                .sync_factory(&args.factory_uid, args.sha)
                .await?;
            match output_format {
                OutputFormat::Json | OutputFormat::Ndjson => {
                    super::output::write_json_line(&accepted, std::io::stdout())?;
                }
                OutputFormat::Pretty | OutputFormat::Text => {
                    println!("Sync accepted for commit {}.", accepted.commit_sha);
                }
            }

            if !args.wait {
                return Ok(());
            }

            if matches!(output_format, OutputFormat::Pretty | OutputFormat::Text) {
                println!("Waiting for sync to finish...");
            }
            let summary = wait_for_factory_sync(
                factory_client.as_ref(),
                &args.factory_uid,
                &accepted.commit_sha,
                WAIT_POLL_INTERVAL,
                WAIT_MAX_POLLS,
            )
            .await?;
            let outcome = apply_wait_outcome(&summary)?;
            match output_format {
                OutputFormat::Json | OutputFormat::Ndjson => {
                    super::output::write_json_line(&summary, std::io::stdout())?;
                }
                OutputFormat::Pretty | OutputFormat::Text => {
                    print!("{outcome}");
                }
            }
            Ok(())
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn init(
        &self,
        args: InitFactoryArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let future = async move {
            let dir = args.dir.unwrap_or_else(|| PathBuf::from("."));
            let files = warp_cli::factory::init_factory_dir(&dir, args.force)?;
            write_written_files(
                &format!("Initialized factory scaffold in {}.", dir.display()),
                &dir,
                &files,
                output_format,
                &mut std::io::stdout(),
            )
        };
        self.spawn_command(future, ctx);
        Ok(())
    }

    fn export(
        &self,
        args: ExportFactoryArgs,
        output_format: OutputFormat,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let factory_client = ServerApiProvider::as_ref(ctx).get_factory_client();
        let future = async move {
            let (dir, files) = export_factory_to_dir(
                factory_client.as_ref(),
                &args.factory_uid,
                args.out,
                args.force,
            )
            .await?;
            write_written_files(
                &format!("Exported factory {} to {}.", args.factory_uid, dir.display()),
                &dir,
                &files,
                output_format,
                &mut std::io::stdout(),
            )
        };
        self.spawn_command(future, ctx);
        Ok(())
    }
}

/// Download a factory's rendered config files and write them under the out
/// directory, defaulting to `./<factory-name>` from the exported factory.yaml.
async fn export_factory_to_dir(
    factory_client: &dyn FactoryClient,
    factory_uid: &str,
    out: Option<PathBuf>,
    force: bool,
) -> anyhow::Result<(PathBuf, Vec<String>)> {
    let export = factory_client.export_factory(factory_uid).await?;
    let dir = match out {
        Some(out) => out,
        None => PathBuf::from(exported_factory_name(&export)?),
    };
    let files = warp_cli::factory::write_export_files(&dir, &export.files, force)?;
    Ok((dir, files))
}

/// The default export directory name: the factory name declared in the
/// exported factory.yaml.
fn exported_factory_name(export: &FactoryExportResponse) -> anyhow::Result<String> {
    #[derive(serde::Deserialize)]
    struct ExportedFactorySpec {
        name: Option<String>,
    }

    let factory_yaml = export.files.get("factory.yaml").ok_or_else(|| {
        anyhow!("The export has no factory.yaml; pass --out to choose an output directory")
    })?;
    let spec: ExportedFactorySpec = serde_yaml::from_str(factory_yaml).map_err(|err| {
        anyhow!(
            "Could not parse the exported factory.yaml ({err}); pass --out to choose an output directory"
        )
    })?;
    let name = spec.name.unwrap_or_default();
    let mut components = Path::new(&name).components();
    if name.is_empty()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        bail!(
            "The exported factory.yaml has no usable name; pass --out to choose an output directory"
        );
    }
    Ok(name)
}

fn write_written_files<W>(
    headline: &str,
    dir: &Path,
    files: &[String],
    output_format: OutputFormat,
    output: &mut W,
) -> anyhow::Result<()>
where
    W: Write,
{
    match output_format {
        OutputFormat::Json => super::output::write_json(&written_files_json(dir, files), output),
        OutputFormat::Ndjson => {
            super::output::write_json_line(&written_files_json(dir, files), output)
        }
        OutputFormat::Pretty | OutputFormat::Text => {
            writeln!(output, "{headline}")?;
            for file in files {
                writeln!(output, "  {file}")?;
            }
            Ok(())
        }
    }
}

fn written_files_json(dir: &Path, files: &[String]) -> serde_json::Value {
    serde_json::json!({ "dir": dir.display().to_string(), "files": files })
}

/// Poll the sync-status endpoint until the sync for `commit_sha` reaches a terminal state.
async fn wait_for_factory_sync(
    factory_client: &dyn FactoryClient,
    factory_uid: &str,
    commit_sha: &str,
    poll_interval: Duration,
    max_polls: usize,
) -> anyhow::Result<FactorySyncSummary> {
    for poll in 0..max_polls {
        let status = factory_client.get_factory_sync_status(factory_uid).await?;
        if let Some(summary) = terminal_sync_for_commit(&status, commit_sha) {
            return Ok(summary);
        }
        if poll + 1 < max_polls {
            Timer::after(poll_interval).await;
        }
    }
    Err(anyhow!(
        "Timed out waiting for the sync of commit {commit_sha} to finish"
    ))
}

/// Returns the terminal sync summary for `commit_sha`, if the latest sync targets that
/// commit and has finished.
fn terminal_sync_for_commit(
    status: &FactorySyncStatusResponse,
    commit_sha: &str,
) -> Option<FactorySyncSummary> {
    let latest = status.latest_sync.as_ref()?;
    if latest.commit_sha != commit_sha || !latest.status.is_terminal() {
        return None;
    }
    Some(latest.clone())
}

/// Map a terminal sync summary to the human-readable outcome for `apply --wait`.
///
/// Failed syncs surface as an error so the process exits non-zero.
fn apply_wait_outcome(summary: &FactorySyncSummary) -> anyhow::Result<String> {
    let mut rendered = String::new();
    match summary.status {
        FactorySyncState::Failed => {
            let mut message = format!("Sync of commit {} failed.", summary.commit_sha);
            if !summary.resource_errors.is_empty() {
                message.push_str("\nErrors:");
                for error in &summary.resource_errors {
                    message.push('\n');
                    message.push_str(&format_resource_error(error));
                }
            }
            return Err(anyhow!(message));
        }
        FactorySyncState::Success => {
            rendered.push_str(&format!(
                "Sync of commit {} succeeded.\n",
                summary.commit_sha
            ));
        }
        FactorySyncState::Noop => {
            rendered.push_str(&format!(
                "Sync of commit {} was a no-op; the factory is already up to date.\n",
                summary.commit_sha
            ));
        }
        FactorySyncState::Partial => {
            rendered.push_str(&format!(
                "Sync of commit {} applied with degraded resources.\n",
                summary.commit_sha
            ));
        }
        FactorySyncState::Pending | FactorySyncState::Running => {
            return Err(anyhow!(
                "Sync of commit {} is still {}",
                summary.commit_sha,
                summary.status.as_str()
            ));
        }
    }
    if !summary.degraded_reasons.is_empty() {
        rendered.push_str("Degraded reasons:\n");
        for reason in &summary.degraded_reasons {
            rendered.push_str(&format!("- {reason}\n"));
        }
    }
    Ok(rendered)
}

fn format_resource_error(error: &FactoryResourceError) -> String {
    match error.line {
        Some(line) => format!("- {}:{}: {}", error.resource_path, line, error.message),
        None => format!("- {}: {}", error.resource_path, error.message),
    }
}

fn format_source(source: &FactorySource) -> String {
    let mut rendered = format!(
        "{}/{} @ {} ({})",
        source.repository.owner, source.repository.repo, source.r#ref, source.code_forge
    );
    if !source.path.is_empty() {
        rendered.push_str(&format!(", path: {}", source.path));
    }
    rendered
}

fn write_linked_factory<W>(
    factory: &FactoryResponse,
    output_format: OutputFormat,
    output: &mut W,
) -> anyhow::Result<()>
where
    W: Write,
{
    match output_format {
        OutputFormat::Json => super::output::write_json(factory, output),
        OutputFormat::Ndjson => super::output::write_json_line(factory, output),
        OutputFormat::Pretty | OutputFormat::Text => {
            writeln!(output, "Factory {} linked.", factory.uid)?;
            if let Some(source) = &factory.source {
                writeln!(output, "Source: {}", format_source(source))?;
            }
            Ok(())
        }
    }
}

fn write_status<W>(
    status: &FactorySyncStatusResponse,
    output_format: OutputFormat,
    output: &mut W,
) -> anyhow::Result<()>
where
    W: Write,
{
    match output_format {
        OutputFormat::Json => super::output::write_json(status, output),
        OutputFormat::Ndjson => super::output::write_json_line(status, output),
        OutputFormat::Pretty | OutputFormat::Text => {
            writeln!(output, "Management mode: {}", status.management_mode)?;
            match &status.source {
                Some(source) => writeln!(output, "Source: {}", format_source(source))?,
                None => writeln!(output, "Source: none")?,
            }
            writeln!(
                output,
                "Last synced commit: {}",
                status.last_synced_commit.as_deref().unwrap_or("none")
            )?;
            match &status.latest_sync {
                Some(latest) => {
                    writeln!(output, "Latest sync:")?;
                    writeln!(output, "  Commit: {}", latest.commit_sha)?;
                    writeln!(output, "  Status: {}", latest.status.as_str())?;
                    writeln!(output, "  Started: {}", latest.started_at.to_rfc3339())?;
                    if let Some(finished_at) = latest.finished_at {
                        writeln!(output, "  Finished: {}", finished_at.to_rfc3339())?;
                    }
                    if !latest.resource_errors.is_empty() {
                        writeln!(output, "  Errors:")?;
                        for error in &latest.resource_errors {
                            writeln!(output, "  {}", format_resource_error(error))?;
                        }
                    }
                    if !latest.degraded_reasons.is_empty() {
                        writeln!(output, "  Degraded reasons:")?;
                        for reason in &latest.degraded_reasons {
                            writeln!(output, "  - {reason}")?;
                        }
                    }
                }
                None => writeln!(output, "Latest sync: never synced")?,
            }
            Ok(())
        }
    }
}

fn write_plan<W>(
    result: &FactorySyncDryRunResult,
    output_format: OutputFormat,
    output: &mut W,
) -> anyhow::Result<()>
where
    W: Write,
{
    match output_format {
        OutputFormat::Json => super::output::write_json(result, output),
        OutputFormat::Ndjson => super::output::write_json_line(result, output),
        OutputFormat::Pretty | OutputFormat::Text => {
            let Some(plan) = &result.plan else {
                let mut message = format!(
                    "Dry-run of commit {} failed with structural errors:",
                    result.commit_sha
                );
                for error in &result.resource_errors {
                    message.push('\n');
                    message.push_str(&format_resource_error(error));
                }
                return Err(anyhow!(message));
            };

            writeln!(output, "Plan for commit {}:", result.commit_sha)?;
            let mut write_group = |label: &str,
                                   marker: char,
                                   changes: &[FactoryPlannedChange]|
             -> anyhow::Result<()> {
                if changes.is_empty() {
                    return Ok(());
                }
                writeln!(output)?;
                writeln!(output, "{label} ({}):", changes.len())?;
                for change in changes {
                    writeln!(
                        output,
                        "{marker} {} ({}): {}",
                        change.path, change.kind, change.reason
                    )?;
                }
                Ok(())
            };
            write_group("Create", '+', &plan.creates)?;
            write_group("Update", '~', &plan.updates)?;
            write_group("Delete", '-', &plan.deletes)?;

            if plan.creates.is_empty() && plan.updates.is_empty() && plan.deletes.is_empty() {
                writeln!(output)?;
                writeln!(output, "No changes.")?;
            }
            writeln!(output)?;
            writeln!(output, "{} resource(s) unchanged.", plan.no_ops)?;
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
