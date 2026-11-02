//! `agent run-cloud-standby`: the entrypoint of a pre-warmed sandbox.
//!
//! The sandbox prepares and heartbeats until the server commits a run to it, installs that run
//! in the same process, reconnects with the run's credential so the server can confirm delivery,
//! and then runs it.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use cloud_object_models::{CodeForge, SourceRepo};
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt as _, Sink, SinkExt as _, StreamExt as _, sink, stream};
use warp_cli::agent::{Harness, RunCloudStandbyArgs};
use warp_cli::skill::SkillSpec;
use warp_core::features::FeatureFlag;
use warp_core::{safe_info, safe_warn};
use warp_graphql::queries::execution_config::ExecutionConfiguration;
use warpui::r#async::Timer;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelSpawner, SingletonEntity};

use crate::ai::agent_sdk::driver::agent_setup::configured_skills::{
    merge_skill_dirs, resolve_configured_skills,
};
use crate::ai::agent_sdk::driver::agent_setup::resolve_global_skills;
use crate::ai::agent_sdk::driver::environment::{WorkspaceConfiguration, merge_repos_deduped};
use crate::ai::agent_sdk::driver::{
    AgentDriver, AgentDriverError, AgentDriverOptions, Task, cloud_provider, git_credentials,
};
use crate::ai::agent_sdk::setup_observability::{SetupClientEventReporter, SetupStep};
use crate::ai::agent_sdk::{AgentDriverRunner, config_file, execution_config, mcp_config};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::GitCredential;
use crate::server::server_api::prewarmed_sandbox::{
    PrewarmedSandboxClient, PrewarmedSandboxConfiguration, PrewarmedSandboxControlEvent,
    PrewarmedSandboxControlStream, PrewarmedSandboxState, is_prewarmed_sandbox_unavailable,
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
const CONTROL_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
/// How long an installed run waits for the server to confirm delivery before giving up, so an
/// unreachable server fails the run promptly rather than holding it until its deadline.
const DELIVERY_CONFIRMATION_WINDOW: Duration = Duration::from_secs(5 * 60);

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

    let clock = StandbyClock::default();
    run_lifecycle(
        &generation,
        expires_at,
        async {
            report_preparation_outcome(
                api.as_ref(),
                &sandbox_id,
                &generation,
                prepare(&foreground, config, &setup_events).await,
            )
            .await
        },
        || {
            async {
                match api.connect_control_stream(&sandbox_id, &generation).await {
                    Ok(connection) => ConnectionAttempt::Connected(control_connection(connection)),
                    Err(error) if is_prewarmed_sandbox_unavailable(&error) => {
                        ConnectionAttempt::Unavailable
                    }
                    Err(_) => ConnectionAttempt::Retry,
                }
            }
            .boxed()
        },
        |driver_spawner, assignment| graduate_sandbox(driver_spawner, assignment).boxed(),
        |(spawner, task)| {
            async move {
                AgentDriver::run_prewarmed_assignment(&spawner, task).await?;
                Ok(())
            }
            .boxed()
        },
        &clock,
    )
    .await
}

/// Reports the terminal preparation outcome and propagates it.
///
/// Reporting a failure is best-effort: the reconciler retires a sandbox that stops
/// heartbeating anyway, so a failed report must not mask the preparation error that caused
/// it.
async fn report_preparation_outcome<T>(
    api: &dyn PrewarmedSandboxClient,
    sandbox_id: &str,
    generation: &str,
    outcome: Result<T, AgentDriverError>,
) -> anyhow::Result<T> {
    match outcome {
        Ok(prepared) => {
            api.report_status(sandbox_id, generation, PrewarmedSandboxState::Ready)
                .await?;
            Ok(prepared)
        }
        Err(error) => {
            log::error!("Pre-warmed sandbox {sandbox_id} preparation failed");
            if api
                .report_status(sandbox_id, generation, PrewarmedSandboxState::Failed)
                .await
                .is_err()
            {
                log::warn!("Could not report the pre-warmed sandbox preparation failure");
            }
            Err(anyhow::Error::new(error).context("Pre-warmed sandbox preparation failed"))
        }
    }
}

/// A live control socket: heartbeats go out and server events come in.
struct ControlConnection {
    /// Sends one heartbeat for the configuration generation it is given.
    heartbeats: Pin<Box<dyn Sink<String, Error = anyhow::Error> + Send>>,
    events: BoxStream<'static, anyhow::Result<PrewarmedSandboxControlEvent>>,
}

enum ConnectionAttempt {
    Connected(ControlConnection),
    /// The server answered `410 Gone`: the sandbox, or its assigned run, is no longer active.
    Unavailable,
    Retry,
}

struct StandbyClock {
    now: Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>,
    sleep: Arc<dyn Fn(Duration) -> BoxFuture<'static, ()> + Send + Sync>,
}

impl Default for StandbyClock {
    fn default() -> Self {
        Self {
            now: Arc::new(Utc::now),
            sleep: Arc::new(|duration| {
                async move {
                    Timer::after(duration).await;
                }
                .boxed()
            }),
        }
    }
}

fn control_connection(connection: PrewarmedSandboxControlStream) -> ControlConnection {
    let (writer, reader) = connection.split();
    ControlConnection {
        heartbeats: Box::pin(sink::unfold(
            writer,
            |mut writer, generation: String| async move {
                writer.send_heartbeat(&generation).await?;
                Ok(writer)
            },
        )),
        events: stream::try_unfold(reader, |mut reader| async move {
            Ok::<_, anyhow::Error>(reader.next().await?.map(|event| (event, reader)))
        })
        .boxed(),
    }
}

async fn bounded_control<T>(
    clock: &StandbyClock,
    operation: impl Future<Output = T>,
) -> anyhow::Result<T> {
    let timeout = (clock.sleep)(CONTROL_REQUEST_TIMEOUT).fuse();
    let operation = operation.fuse();
    futures::pin_mut!(timeout, operation);
    futures::select_biased! {
        () = timeout => anyhow::bail!("Pre-warmed sandbox control request timed out"),
        result = operation => Ok(result),
    }
}

/// Keeps a standby control socket open, heartbeating and forwarding server events, until the
/// server reports the sandbox unavailable.
async fn maintain_standby_connection<'a>(
    generation: &str,
    connect: &mut impl FnMut() -> BoxFuture<'a, ConnectionAttempt>,
    events: async_channel::Sender<PrewarmedSandboxControlEvent>,
    clock: &StandbyClock,
) -> anyhow::Result<()> {
    loop {
        let ControlConnection {
            mut heartbeats,
            events: mut incoming,
        } = match bounded_control(clock, connect()).await {
            Ok(ConnectionAttempt::Connected(connection)) => connection,
            Ok(ConnectionAttempt::Unavailable) => {
                anyhow::bail!("Pre-warmed sandbox is unavailable")
            }
            Ok(ConnectionAttempt::Retry) | Err(_) => {
                (clock.sleep)(RECONNECT_DELAY).await;
                continue;
            }
        };
        let mut heartbeat = (clock.sleep)(HEARTBEAT_INTERVAL).fuse();
        loop {
            futures::select_biased! {
                () = heartbeat => {
                    let sent = bounded_control(clock, heartbeats.send(generation.to_owned())).await;
                    if !matches!(sent, Ok(Ok(()))) {
                        break;
                    }
                    heartbeat = (clock.sleep)(HEARTBEAT_INTERVAL).fuse();
                },
                event = incoming.next().fuse() => match event {
                    Some(Ok(event)) => events
                        .send(event)
                        .await
                        .map_err(|_| anyhow::anyhow!("Standby event queue closed"))?,
                    Some(Err(_)) | None => break,
                },
            }
        }
        (clock.sleep)(RECONNECT_DELAY).await;
    }
}

struct Assignment {
    execution_config: ExecutionConfiguration,
    runtime_env: HashMap<String, String>,
    api_key: String,
    expires_at: DateTime<Utc>,
}

/// Waits until both preparation and the assignment are in hand. Returns `None` when the server
/// retires the sandbox instead.
///
/// The server assigns only a sandbox that reported ready, but the assignment can still arrive
/// before the ready report's response does, so an early assignment is held rather than refused.
async fn wait_for_assignment<P>(
    generation: &str,
    preparation: impl Future<Output = anyhow::Result<P>>,
    events: &async_channel::Receiver<PrewarmedSandboxControlEvent>,
    clock: &StandbyClock,
) -> anyhow::Result<Option<(P, Assignment)>> {
    let preparation = preparation.fuse();
    futures::pin_mut!(preparation);
    let mut prepared = None;
    let mut held: Option<Assignment> = None;
    loop {
        if prepared.is_some() && held.is_some() {
            return Ok(prepared.zip(held));
        }
        futures::select_biased! {
            result = preparation => prepared = Some(result?),
            event = events.recv().fuse() => match event.context("Standby control ended")? {
                PrewarmedSandboxControlEvent::Assignment {
                    generation: assigned_generation,
                    execution_config,
                    runtime_env,
                    api_key,
                    expires_at,
                } => {
                    if let Some(held) = &held {
                        anyhow::ensure!(
                            held.execution_config.execution_id == execution_config.execution_id,
                            "A second assignment arrived before the first was installed"
                        );
                        continue;
                    }
                    anyhow::ensure!(
                        assigned_generation == generation,
                        "Assignment does not match the prepared generation"
                    );
                    anyhow::ensure!(expires_at > (clock.now)(), "Assignment deadline has expired");
                    held = Some(Assignment {
                        execution_config: *execution_config,
                        runtime_env,
                        api_key,
                        expires_at,
                    });
                }
                PrewarmedSandboxControlEvent::Retired { .. } => return Ok(None),
                PrewarmedSandboxControlEvent::Unknown => {
                    log::warn!("Ignoring an unknown pre-warmed sandbox control message");
                }
            },
        }
    }
}

/// Runs the standby phase until an assignment is ready to install. Returns `None` when the
/// sandbox is retired or reaches its standby deadline first.
async fn wait_in_standby<'a, P>(
    generation: &str,
    expires_at: DateTime<Utc>,
    preparation: impl Future<Output = anyhow::Result<P>>,
    connect: &mut impl FnMut() -> BoxFuture<'a, ConnectionAttempt>,
    clock: &StandbyClock,
) -> anyhow::Result<Option<(P, Assignment)>> {
    let (event_tx, events) = async_channel::unbounded();
    let transport = maintain_standby_connection(generation, connect, event_tx, clock).fuse();
    let waiting = wait_for_assignment(generation, preparation, &events, clock).fuse();
    let expiry = (clock.sleep)(wait_window(expires_at, (clock.now)())).fuse();
    futures::pin_mut!(transport, waiting, expiry);
    futures::select_biased! {
        () = expiry => Ok(None),
        result = waiting => result,
        result = transport => {
            result?;
            anyhow::bail!("Standby transport ended")
        },
    }
}

/// Reconnects with the installed run credential so the server can confirm that the run reached
/// this sandbox, which is also when it revokes the standby credential. Returns `false` when the
/// server reports the run is no longer active, in which case it must not start.
async fn confirm_delivery<'a>(
    execution_id: &str,
    connect: &mut impl FnMut() -> BoxFuture<'a, ConnectionAttempt>,
    clock: &StandbyClock,
) -> anyhow::Result<bool> {
    loop {
        match bounded_control(clock, connect()).await {
            Ok(ConnectionAttempt::Connected(mut connection)) => {
                // Every admitted connection replays the assignment, or retires the socket.
                loop {
                    match bounded_control(clock, connection.events.next()).await {
                        Ok(Some(Ok(PrewarmedSandboxControlEvent::Assignment {
                            execution_config: replayed,
                            ..
                        }))) => {
                            anyhow::ensure!(
                                replayed.execution_id.inner() == execution_id,
                                "The server replayed a different assignment"
                            );
                            return Ok(true);
                        }
                        Ok(Some(Ok(PrewarmedSandboxControlEvent::Retired { .. }))) => {
                            return Ok(false);
                        }
                        Ok(Some(Ok(PrewarmedSandboxControlEvent::Unknown))) => {}
                        Ok(Some(Err(_)) | None) | Err(_) => break,
                    }
                }
            }
            Ok(ConnectionAttempt::Unavailable) => return Ok(false),
            Ok(ConnectionAttempt::Retry) | Err(_) => {}
        }
        (clock.sleep)(RECONNECT_DELAY).await;
    }
}

/// Waits in standby for an assignment, installs it, confirms its delivery, and then runs it.
async fn run_lifecycle<'a, P, R>(
    generation: &str,
    expires_at: DateTime<Utc>,
    preparation: impl Future<Output = anyhow::Result<P>>,
    mut connect: impl FnMut() -> BoxFuture<'a, ConnectionAttempt>,
    install: impl FnOnce(P, Assignment) -> BoxFuture<'a, anyhow::Result<R>>,
    run: impl FnOnce(R) -> BoxFuture<'a, anyhow::Result<()>>,
    clock: &StandbyClock,
) -> anyhow::Result<()> {
    let Some((prepared, assignment)) =
        wait_in_standby(generation, expires_at, preparation, &mut connect, clock).await?
    else {
        return Ok(());
    };
    let execution_id = assignment.execution_config.execution_id.inner().to_owned();
    let deadline = (clock.sleep)(wait_window(assignment.expires_at, (clock.now)())).fuse();
    futures::pin_mut!(deadline);
    let installed = {
        let install = install(prepared, assignment).fuse();
        futures::pin_mut!(install);
        futures::select_biased! {
            () = deadline => anyhow::bail!("Assigned sandbox deadline expired during installation"),
            result = install => result?,
        }
    };
    let delivered = {
        let confirm = confirm_delivery(&execution_id, &mut connect, clock).fuse();
        let window = (clock.sleep)(DELIVERY_CONFIRMATION_WINDOW).fuse();
        futures::pin_mut!(confirm, window);
        futures::select_biased! {
            () = deadline => anyhow::bail!("Assigned sandbox deadline expired before delivery was confirmed"),
            () = window => anyhow::bail!("The server did not confirm delivery of the assignment"),
            result = confirm => result?,
        }
    };
    if !delivered {
        safe_info!(
            safe: ("Pre-warmed sandbox run is no longer active; exiting without running it"),
            full: ("Pre-warmed sandbox assignment for execution {execution_id} is no longer active; exiting without running it")
        );
        return Ok(());
    }
    run(installed).await
}

async fn graduate_sandbox(
    driver_spawner: ModelSpawner<AgentDriver>,
    assignment: Assignment,
) -> anyhow::Result<(ModelSpawner<AgentDriver>, Task)> {
    let task = AgentDriver::prepare_prewarmed_assignment(
        &driver_spawner,
        assignment.execution_config,
        assignment.runtime_env,
        assignment.api_key,
        assignment.expires_at,
    )
    .await
    .context("Failed to prepare pre-warmed sandbox assignment")?;
    Ok((driver_spawner, task))
}

/// How long to wait before exiting, given a deadline. Saturates at zero so an already-expired
/// sandbox exits immediately instead of waiting out an underflowed duration.
fn wait_window(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> Duration {
    (expires_at - now)
        .to_std()
        .unwrap_or(Duration::ZERO)
        .saturating_sub(EXPIRY_MARGIN)
}

/// Prepares a sandbox through the shared driver without a prompt or run-only setup.
async fn prepare(
    foreground: &ModelSpawner<AgentDriverRunner>,
    config: PrewarmedSandboxConfiguration,
    setup_events: &SetupClientEventReporter,
) -> Result<ModelSpawner<AgentDriver>, AgentDriverError> {
    let preparation = config.preparation;
    let harness = Harness::from_config_name(&preparation.harness)
        .with_context(|| {
            format!(
                "Unknown pre-warmed sandbox harness '{}'",
                preparation.harness,
            )
        })
        .map_err(AgentDriverError::ConfigBuildFailed)?;
    execution_config::validate_prewarmed_harness(harness)
        .map_err(AgentDriverError::ConfigBuildFailed)?;
    // Deferred checkouts belong to the claiming task, so the standby workspace declares none and
    // only validates them.
    let mut workspace = WorkspaceConfiguration::from_resolved(
        execution_config::repositories(preparation.repositories)
            .map_err(AgentDriverError::ConfigBuildFailed)?,
        Vec::new(),
        preparation.setup_commands,
    )
    .map_err(|error| AgentDriverError::ConfigBuildFailed(error.into()))?;
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
    let cloud_providers = if FeatureFlag::OzIdentityFederation.is_enabled() {
        cloud_provider::load_providers(&execution_config::providers(preparation.providers), None)
            .map_err(AgentDriverError::CloudProviderSetupFailed)?
    } else {
        Vec::new()
    };
    let options = AgentDriverOptions {
        working_dir,
        secrets,
        environment_variables: config.environment_variables,
        workspace,
        cloud_providers,
        computer_use_config: (harness == Harness::Oz)
            .then_some((preparation.computer_use_enabled, None)),
        selected_harness: harness,
        ..Default::default()
    };
    let driver_spawner = foreground
        .spawn(move |_, ctx| {
            let driver = AgentDriver::create(options, ctx)?;
            Ok::<_, AgentDriverError>(driver.update(ctx, |_, ctx| ctx.spawner()))
        })
        .await??;
    AgentDriver::prepare_resources(
        &driver_spawner,
        harness,
        mcp_specs,
        global_skills,
        setup_events,
    )
    .await?;
    Ok(driver_spawner)
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
