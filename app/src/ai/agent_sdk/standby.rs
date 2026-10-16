//! `agent run-cloud-standby`: the entrypoint of a pre-warmed sandbox.
//!
//! The sandbox prepares task-free resources while maintaining its lease, then waits until expiry.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use cloud_object_models::{CodeForge, SourceRepo};
use futures::channel::oneshot;
use futures::{FutureExt as _, Stream, StreamExt as _, future, stream};
use warp_cli::agent::{Harness, RunCloudStandbyArgs};
use warp_cli::skill::SkillSpec;
use warp_core::{safe_info, safe_warn};
use warpui::r#async::Timer;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelSpawner, SingletonEntity};

use crate::ai::agent_sdk::driver::agent_setup::configured_skills::{
    merge_skill_dirs, resolve_configured_skills,
};
use crate::ai::agent_sdk::driver::agent_setup::resolve_global_skills;
use crate::ai::agent_sdk::driver::environment::{WorkspaceConfiguration, merge_repos_deduped};
use crate::ai::agent_sdk::driver::{
    AgentDriver, AgentDriverError, AgentDriverOptions, IdleTimeoutSender, git_credentials,
};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::agent_sdk::{AgentDriverRunner, config_file, execution_config, mcp_config};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::GitCredential;
use crate::server::server_api::prewarmed_sandbox::{
    PrewarmedSandboxClient, PrewarmedSandboxConfiguration, PrewarmedSandboxState,
};

/// Environment variable the allocator sets to the directory repositories are cloned under.
const WORKSPACE_ROOT_ENV: &str = "OZ_WORKSPACE_ROOT";

/// How long the server goes without a heartbeat before it treats a sandbox as unhealthy.
const SERVER_HEARTBEAT_TOLERANCE: Duration = Duration::from_secs(3 * 60);

/// How often the sandbox heartbeats. Derived from the server's tolerance so two consecutive
/// failures still leave room for a third attempt before the reconciler retires a healthy
/// sandbox.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(SERVER_HEARTBEAT_TOLERANCE.as_secs() / 6);

/// How far before its deadline the sandbox stops waiting, so the process exits cleanly rather
/// than being killed mid-request when the Namespace instance deadline lands.
const EXPIRY_MARGIN: Duration = Duration::from_secs(15);

/// Keeps the results of standby preparation for a future run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreparedSandbox {
    /// Identifies the configuration that preparation completed.
    pub generation: String,
    /// Records the actual harness directory, which depends on the number of cloned repositories.
    pub harness_working_dir: PathBuf,
    /// Lists repositories for later on-demand checkout without cloning them during preparation.
    pub deferred_repositories: Vec<SourceRepo>,
}

/// Why the standby stopped waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StandbyExit {
    /// The sandbox reached its deadline without being claimed.
    Expired,
}

/// Starts the standby lifecycle and exits the app after expiry or a fatal error.
pub(super) fn run(ctx: &mut AppContext, args: RunCloudStandbyArgs) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_| AgentDriverRunner);
    runner.update(ctx, move |_, ctx| {
        let spawner = ctx.spawner();
        ctx.spawn(
            prepare_and_wait(spawner, args.standby_id),
            |_, result, ctx| match result {
                Ok(()) => ctx.terminate_app(TerminationMode::ForceTerminate, None),
                Err(err) => super::report_fatal_error(err, ctx),
            },
        );
    });
    Ok(())
}

/// Fetches one configuration and keeps its sandbox alive through preparation and the idle wait.
// TODO(REMOTE-3488): Enable sandbox-scoped OTLP export for standby preparation spans.
#[tracing::instrument(name = "standby::prepare_and_wait", skip_all, err, fields(tags.cloud_agent = true))]
async fn prepare_and_wait(
    foreground: ModelSpawner<AgentDriverRunner>,
    sandbox_id: String,
) -> anyhow::Result<()> {
    let api = foreground
        .spawn(|_, ctx| ServerApiProvider::as_ref(ctx).get_prewarmed_sandbox_client())
        .await?;
    let setup_events = foreground
        .spawn(|_, ctx| {
            let ai_client = ServerApiProvider::as_ref(ctx).get_ai_client().clone();
            SetupClientEventReporter::standby(ai_client, ctx.background_executor())
        })
        .await?;

    let config = api.fetch_configuration(&sandbox_id).await?;
    let generation = config.generation.clone();
    let expires_at = config.expires_at;
    safe_info!(
        safe: ("Preparing pre-warmed sandbox {sandbox_id}"),
        full: (
            "Preparing pre-warmed sandbox {sandbox_id} generation={generation} \
             repositories={} setup_commands={} expires_at={expires_at}",
            config.preparation.repositories.len(),
            config.preparation.setup_commands.len()
        )
    );

    let (tx, exit_rx) = oneshot::channel();
    let exit = IdleTimeoutSender::new(tx);
    exit.end_run_after(wait_window(expires_at, Utc::now()), StandbyExit::Expired);
    let ticks = stream::repeat(()).then(|()| async {
        Timer::after(HEARTBEAT_INTERVAL).await;
    });
    maintain_sandbox(
        api.as_ref(),
        &sandbox_id,
        &generation,
        prepare(&foreground, config, &setup_events),
        exit_rx.map(|reason| reason.unwrap_or(StandbyExit::Expired)),
        ticks,
    )
    .await
}

/// Reports the terminal preparation outcome and propagates it.
///
/// Reporting a failure is best-effort: the reconciler retires a sandbox that stops
/// heartbeating anyway, so a failed report must not mask the preparation error that caused
/// it.
async fn report_preparation_outcome(
    api: &dyn PrewarmedSandboxClient,
    sandbox_id: &str,
    generation: &str,
    outcome: Result<PreparedSandbox, AgentDriverError>,
) -> anyhow::Result<PreparedSandbox> {
    let error = match outcome {
        Ok(prepared) => {
            api.report_status(sandbox_id, generation, PrewarmedSandboxState::Ready)
                .await?;
            return Ok(prepared);
        }
        Err(error) => error,
    };

    log::error!("Pre-warmed sandbox {sandbox_id} preparation failed: {error:#}");
    if let Err(report_error) = api
        .report_status(sandbox_id, generation, PrewarmedSandboxState::Failed)
        .await
    {
        log::warn!("Could not report the pre-warmed sandbox preparation failure: {report_error:#}");
    }
    Err(anyhow::Error::new(error).context("Pre-warmed sandbox preparation failed"))
}

/// Keeps heartbeats active during preparation and status reporting, with expiry taking priority.
async fn maintain_sandbox(
    api: &dyn PrewarmedSandboxClient,
    sandbox_id: &str,
    generation: &str,
    preparation: impl Future<Output = Result<PreparedSandbox, AgentDriverError>>,
    exit: impl Future<Output = StandbyExit>,
    ticks: impl Stream<Item = ()>,
) -> anyhow::Result<()> {
    let lifecycle = async {
        let prepared =
            report_preparation_outcome(api, sandbox_id, generation, preparation.await).await?;
        log::info!(
            "Pre-warmed sandbox {sandbox_id} is ready; harness working directory is {}",
            prepared.harness_working_dir.display()
        );
        future::pending::<anyhow::Result<()>>().await
    };

    let heartbeats = async {
        futures::pin_mut!(ticks);
        while ticks.next().await.is_some() {
            if let Err(error) = api.heartbeat(sandbox_id, generation).await {
                log::warn!("Pre-warmed sandbox heartbeat failed: {error:#}");
            }
        }
    };
    let exit = exit.fuse();
    let lifecycle = lifecycle.fuse();
    let heartbeats = heartbeats.fuse();
    futures::pin_mut!(exit, lifecycle, heartbeats);
    futures::select_biased! {
        reason = exit => match reason {
            StandbyExit::Expired => {
                // The server owns expires_at; another request could overrun the exit margin.
                log::info!("Pre-warmed sandbox {sandbox_id} reached its deadline; exiting");
                Ok(())
            }
        },
        result = lifecycle => result,
        () = heartbeats => Err(anyhow::anyhow!("Pre-warmed sandbox heartbeat schedule ended")),
    }
}

/// How long to wait before exiting, given a deadline. Saturates at zero so an already-expired
/// sandbox exits immediately instead of waiting out an underflowed duration.
fn wait_window(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    (expires_at - now)
        .to_std()
        .unwrap_or(Duration::ZERO)
        .saturating_sub(EXPIRY_MARGIN)
}

/// Prepares an Oz sandbox through the shared driver without a prompt or run-only setup.
async fn prepare(
    foreground: &ModelSpawner<AgentDriverRunner>,
    config: PrewarmedSandboxConfiguration,
    setup_events: &SetupClientEventReporter,
) -> Result<PreparedSandbox, AgentDriverError> {
    let preparation = config.preparation;
    let harness = Harness::from_config_name(&preparation.harness)
        .filter(|harness| *harness == Harness::Oz)
        .with_context(|| {
            format!(
                "Pre-warmed sandboxes support only the Oz harness, not '{}'",
                preparation.harness,
            )
        })
        .map_err(AgentDriverError::ConfigBuildFailed)?;
    // Deferred checkouts belong to the claiming task, so the standby workspace declares none and
    // carries them on the prepared sandbox instead.
    let mut workspace = WorkspaceConfiguration::from_resolved(
        execution_config::repositories(preparation.repositories)
            .map_err(AgentDriverError::ConfigBuildFailed)?,
        Vec::new(),
        preparation.setup_commands,
    )
    .map_err(|error| AgentDriverError::ConfigBuildFailed(error.into()))?;
    let deferred_repositories =
        execution_config::deferred_repositories(preparation.deferred_repositories)
            .map_err(AgentDriverError::ConfigBuildFailed)?;
    let mut skill_dirs = execution_config::factory_skill_dirs(preparation.factory_skill_dirs)
        .map_err(AgentDriverError::ConfigBuildFailed)?;
    mcp_config::validate_mcp_servers(&preparation.mcp_servers)
        .map_err(AgentDriverError::ConfigBuildFailed)?;
    let mcp_specs = config_file::mcp_specs_from_mcp_servers(&preparation.mcp_servers)
        .map_err(AgentDriverError::ConfigBuildFailed)?;
    let skills = preparation
        .skills
        .into_iter()
        .map(|spec| spec.parse::<SkillSpec>().map_err(anyhow::Error::msg))
        .collect::<anyhow::Result<Vec<_>>>()
        .context("Invalid pre-warmed sandbox skill spec")
        .map_err(AgentDriverError::ConfigBuildFailed)?;
    let skill_repositories = skills
        .iter()
        .filter_map(|spec| {
            Some(SourceRepo::new(
                CodeForge::GitHub,
                spec.org.clone()?,
                spec.repo.clone()?,
            ))
        })
        .collect();
    // Skill clones share checkout directories with workspace repositories.
    let preparation_repositories =
        merge_repos_deduped(workspace.source_repos.clone(), skill_repositories)
            .map_err(|error| AgentDriverError::ConfigBuildFailed(error.into()))?;
    let global_skills = setup_events
        .record_result(
            SetupStep::GlobalSkillResolution,
            resolve_global_skills(foreground),
        )
        .await?;
    merge_repos_deduped(
        preparation_repositories,
        global_skills.repos.iter().map(SourceRepo::from).collect(),
    )
    .map_err(|error| AgentDriverError::ConfigBuildFailed(error.into()))?;
    let secrets = config.managed_secrets;
    let working_dir = workspace_root().map_err(AgentDriverError::ConfigBuildFailed)?;

    setup_events
        .record_result(SetupStep::TeamMetadataRefresh, async {
            foreground
                .spawn(|_, ctx| {
                    Box::pin(super::common::refresh_workspace_metadata(ctx))
                        as std::pin::Pin<
                            Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>,
                        >
                })
                .await?
                .await
                .map_err(AgentDriverError::TeamMetadataRefreshFailed)
        })
        .await?;
    setup_events
        .record_result(SetupStep::WarpDriveSync, async {
            if foreground
                .spawn(|_, ctx| super::common::refresh_warp_drive(ctx))
                .await?
                .await
                .is_err()
            {
                return Err(AgentDriverError::WarpDriveSyncFailed);
            }
            Ok(())
        })
        .await?;

    setup_events
        .record_result(SetupStep::AgentCredentialsFetch, async {
            apply_git_credentials(&config.git_credentials)
        })
        .await?;

    let (_, discovery_dirs) =
        resolve_configured_skills(foreground, skills, true, &working_dir, setup_events).await?;
    merge_skill_dirs(&mut skill_dirs, discovery_dirs);
    workspace.factory_skill_dirs = Some(skill_dirs);
    let options = AgentDriverOptions {
        working_dir,
        secrets,
        environment_variables: config.environment_variables,
        workspace,
        computer_use_config: Some((preparation.computer_use_enabled, None)),
        selected_harness: harness,
        ..Default::default()
    };
    let driver_spawner = foreground
        .spawn(move |_, ctx| {
            let driver = AgentDriver::create(options, ctx)?;
            Ok::<_, AgentDriverError>(driver.update(ctx, |_, ctx| ctx.spawner()))
        })
        .await??;
    let harness_working_dir =
        AgentDriver::prepare_resources(&driver_spawner, mcp_specs, global_skills, setup_events)
            .await?;

    Ok(PreparedSandbox {
        generation: config.generation,
        harness_working_dir,
        deferred_repositories,
    })
}

/// Installs the standby configuration's Git credentials before any repository or skill clone.
fn apply_git_credentials(credentials: &[GitCredential]) -> Result<(), AgentDriverError> {
    if credentials.is_empty() {
        return Ok(());
    }
    git_credentials::configure_git_credentials(credentials).map_err(|error| {
        safe_warn!(
            safe: ("Failed to write pre-warmed sandbox git credentials"),
            full: ("Failed to write pre-warmed sandbox git credentials: {error:#}")
        );
        AgentDriverError::EnvironmentSetupFailed(
            "Failed to write the pre-warmed sandbox git credentials".to_string(),
        )
    })
}

/// Uses the allocator's workspace root, or the process directory if no root was supplied.
fn workspace_root() -> anyhow::Result<PathBuf> {
    match std::env::var_os(WORKSPACE_ROOT_ENV) {
        Some(root) if !root.is_empty() => Ok(PathBuf::from(root)),
        _ => std::env::current_dir()
            .context("Could not determine the pre-warmed sandbox working directory"),
    }
}

#[cfg(test)]
#[path = "standby_tests.rs"]
mod tests;
