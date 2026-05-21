use std::{
    borrow::Cow,
    collections::HashMap,
    ffi::OsString,
    future::Future,
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use crate::ai::agent_sdk::driver::harness::{
    HarnessKind, HarnessRunner, SavePoint, ThirdPartyHarness,
};
use crate::terminal::cli_agent_sessions::plugin_manager::{
    plugin_manager_for, CliAgentPluginManager,
};
use crate::terminal::cli_agent_sessions::{
    CLIAgentSessionStatus, CLIAgentSessionsModel, CLIAgentSessionsModelEvent,
};
use anyhow::Context as _;
use futures::{channel::oneshot, FutureExt as _};
use oneshot::Canceled;
use warp_core::{report_if_error, safe_debug, safe_info};
use warp_managed_secrets::ManagedSecretValue;
use warpui::{Entity, ModelContext, ModelHandle, ModelSpawner, SingletonEntity};

pub(crate) mod harness;
pub(crate) mod terminal;

use terminal::TerminalDriverEvent;

const HARNESS_SAVE_INTERVAL: Duration = Duration::from_secs(30);
/// IdleTimeoutSender is wrapper around a sender that signals when a run is done after
/// an idle timeout. Used for local and third-party harnesses.
///
/// We use a generation-based approach to cancel timers instead of storing timer handles:
///
/// - `tx_cell` holds the completion sender; taking it ensures we only complete once.
/// - `timer_generation` starts at 0 and is incremented each time we want to cancel
///   existing timers and potentially start a new one. When a timer fires, it checks
///   if its generation still matches the current generation. If not, the timer was
///   "cancelled" by a newer timer and should not complete the conversation.
///
/// This approach avoids the complexity of storing and cancelling timer handles,
/// while allowing multiple events to safely race without double-completion.
struct IdleTimeoutSender<T: Send + 'static> {
    tx_cell: Arc<Mutex<Option<oneshot::Sender<T>>>>,
    generation: Arc<AtomicUsize>,
}

impl<T: Send + 'static> IdleTimeoutSender<T> {
    fn new(tx: oneshot::Sender<T>) -> Self {
        Self {
            tx_cell: Arc::new(Mutex::new(Some(tx))),
            generation: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// End the run by sending `value` immediately.
    fn end_run_now(&self, value: T) {
        if let Ok(mut guard) = self.tx_cell.lock() {
            if let Some(sender) = guard.take() {
                let _ = sender.send(value);
            }
        }
    }

    /// End the run after `timeout` by sending `value`, unless cancelled before then.
    fn end_run_after(&self, timeout: Duration, value: T) {
        // Increment the generation counter to invalidate any existing timers,
        // then capture the new generation for our timer to check against.
        let current_gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let tx_cell = Arc::clone(&self.tx_cell);
        let generation = Arc::clone(&self.generation);

        // Spawn a background thread that will complete the oneshot after the idle timeout,
        // unless a follow-up query resets the timer (by bumping the generation counter).
        thread::spawn(move || {
            thread::sleep(timeout);

            // Check if our timer generation is still current. If not, a follow-up
            // query or other activity has "cancelled" this timer by bumping the generation.
            if generation.load(Ordering::SeqCst) != current_gen {
                return;
            }
            if let Ok(mut guard) = tx_cell.lock() {
                if let Some(sender) = guard.take() {
                    // Send the value after the idle timeout expires.
                    let _ = sender.send(value);
                }
            }
        });
    }

    /// Cancel any pending idle timers.
    fn cancel_idle_timeout(&self) {
        if self.generation.load(Ordering::SeqCst) > 0 {
            self.generation.fetch_add(1, Ordering::SeqCst);
        }
    }
}

/// Options for initializing the agent driver.
pub struct AgentDriverOptions {
    /// Initial working directory for the agent's terminal session.
    pub working_dir: PathBuf,
    /// Secrets to inject into the agent's terminal session.
    pub secrets: HashMap<String, ManagedSecretValue>,
    /// How long to keep the session alive after the agent run completes, if at all.
    pub idle_on_complete: Option<Duration>,
}

/// `AgentDriver` is a model for driving an ambient Warp agent to completion.
///
/// Its primary responsibility is to configure a headless terminal pane and execute an AI query within it.
pub struct AgentDriver {
    terminal_driver: ModelHandle<terminal::TerminalDriver>,
    working_dir: PathBuf,

    /// Secrets available to the running agent.
    /// - Secrets are injected as environment variables when the terminal session is created.
    /// - Secrets are passed to MCP servers during spawning.
    secrets: Arc<HashMap<String, ManagedSecretValue>>,

    /// Harness adapter for the running agent. This is only set if:
    /// - The harness has started successfully.
    /// - We're using a third-party harness.
    harness: Option<Arc<dyn HarnessRunner>>,

    // Optional idle timeout after completion. If set, the process will stay alive for follow-ups
    // and exit after this period of inactivity.
    idle_on_complete: Option<Duration>,
}

/// Task configuration for running an agent.
#[derive(Debug)]
pub struct Task {
    /// The prompt for the agent.
    pub prompt: AgentRunPrompt,
    /// Which harness to use for executing the agent run.
    pub harness: HarnessKind,
}

/// Prompt that we initialize an agent driver with.
#[derive(Debug, Clone)]
pub enum AgentRunPrompt {
    /// Prompt is provided locally (already resolved to a plain string).
    Local(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AgentDriverError {
    #[error("Terminal session is not available.")]
    TerminalUnavailable,
    #[error("Invalid runtime state - please file a bug report.")]
    InvalidRuntimeState,
    #[error("Saved prompt not found for id {0}")]
    AIWorkflowNotFound(String),
    #[error("Terminal bootstrap failed")]
    BootstrapFailed,
    #[error("Could not resolve working directory {}", path.display())]
    InvalidWorkingDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{0}")]
    SkillResolutionFailed(String),
    #[error("Failed to build agent configuration")]
    ConfigBuildFailed(#[source] anyhow::Error),
    #[error("Harness command exited with code {exit_code}")]
    HarnessCommandFailed { exit_code: i32 },
    #[error("Harness '{harness}' setup failed: {reason}")]
    HarnessSetupFailed { harness: String, reason: String },
    #[error("Harness '{harness}' config setup failed")]
    HarnessConfigSetupFailed {
        harness: String,
        #[source]
        error: anyhow::Error,
    },
}

impl From<warpui::ModelDropped> for AgentDriverError {
    fn from(_: warpui::ModelDropped) -> Self {
        AgentDriverError::InvalidRuntimeState
    }
}

impl AgentDriver {
    pub fn new(
        options: AgentDriverOptions,
        ctx: &mut ModelContext<Self>,
    ) -> Result<Self, AgentDriverError> {
        let AgentDriverOptions {
            working_dir,
            idle_on_complete,
            secrets,
        } = options;

        safe_info!(
            safe: ("Initializing agent driver: idle_on_complete={idle_on_complete:?}"),
            full: (
                "Initializing agent driver: idle_on_complete={idle_on_complete:?}, working_dir={}",
                working_dir.display()
            )
        );

        // Build environment variables from secrets for the terminal session.
        // Do not override env vars that are already set to a non-empty value in the current
        // process. This ensures that worker-injected credentials (e.g. harness auth secrets)
        // and user-provided env vars (e.g. on self-hosted workers) take precedence over
        // generic managed secrets.
        let mut env_vars = HashMap::with_capacity(secrets.len() + 1);
        for (name, secret) in &secrets {
            let (env_name, env_value) = match secret {
                ManagedSecretValue::RawValue { value } => (name.as_str(), value.as_str()),
                ManagedSecretValue::AnthropicApiKey { api_key } => {
                    ("ANTHROPIC_API_KEY", api_key.as_str())
                }
                ManagedSecretValue::AnthropicBedrockAccessKey {
                    aws_access_key_id,
                    aws_secret_access_key,
                    aws_session_token,
                    aws_region,
                } => {
                    // Inject env vars needed for Claude Code Bedrock access key authentication.
                    // AWS_SESSION_TOKEN is only injected when the user provided one (i.e. for
                    // temporary/STS credentials).
                    let mut vars = vec![
                        ("AWS_ACCESS_KEY_ID", aws_access_key_id.as_str()),
                        ("AWS_SECRET_ACCESS_KEY", aws_secret_access_key.as_str()),
                        ("CLAUDE_CODE_USE_BEDROCK", "1"),
                        ("AWS_REGION", aws_region.as_str()),
                    ];
                    if let Some(token) = aws_session_token.as_deref() {
                        vars.push(("AWS_SESSION_TOKEN", token));
                    }
                    for (env_name, env_value) in vars {
                        if std::env::var(env_name).is_ok_and(|v| !v.is_empty()) {
                            log::warn!(
                                "Skipping managed secret {env_name}: already set in environment"
                            );
                            continue;
                        }
                        env_vars.insert(OsString::from(env_name), OsString::from(env_value));
                    }
                    continue; // Skip the single-var insert below since we handled all vars inline.
                }
                ManagedSecretValue::AnthropicBedrockApiKey {
                    aws_bearer_token_bedrock,
                    aws_region,
                } => {
                    // Inject all three env vars needed for Claude Code Bedrock authentication.
                    let vars = [
                        (
                            "AWS_BEARER_TOKEN_BEDROCK",
                            aws_bearer_token_bedrock.as_str(),
                        ),
                        ("CLAUDE_CODE_USE_BEDROCK", "1"),
                        ("AWS_REGION", aws_region.as_str()),
                    ];
                    for (env_name, env_value) in vars {
                        if std::env::var(env_name).is_ok_and(|v| !v.is_empty()) {
                            log::warn!(
                                "Skipping managed secret {env_name}: already set in environment"
                            );
                            continue;
                        }
                        env_vars.insert(OsString::from(env_name), OsString::from(env_value));
                    }
                    continue; // Skip the single-var insert below since we handled all vars inline.
                }
            };
            if std::env::var(env_name).is_ok_and(|v| !v.is_empty()) {
                log::warn!("Skipping managed secret {env_name}: already set in environment");
                continue;
            }
            env_vars.insert(OsString::from(env_name), OsString::from(env_value));
        }

        // Signal to third-party harnesses (e.g. Claude Code) that we're in a sandbox
        // so they allow root execution with permissive flags.
        if warp_isolation_platform::detect().is_some() {
            env_vars.insert(OsString::from("IS_SANDBOX"), OsString::from("1"));
        }

        let terminal_driver = terminal::TerminalDriver::create(
            terminal::TerminalDriverOptions {
                working_dir: working_dir.clone(),
                env_vars,
                conversation_restoration: None,
            },
            ctx,
        )?;

        // Subscribe to TerminalDriver events for task-specific handling.
        ctx.subscribe_to_model(&terminal_driver, |me, event, ctx| {
            me.handle_terminal_driver_event(event, ctx);
        });

        Ok(Self {
            terminal_driver,
            working_dir,
            secrets: Arc::new(secrets),
            harness: None,
            idle_on_complete,
        })
    }

    pub fn run(
        &mut self,
        task: Task,
        ctx: &mut ModelContext<Self>,
    ) -> impl Future<Output = Result<(), AgentDriverError>> {
        let (tx, rx) = oneshot::channel();
        let foreground = ctx.spawner();

        ctx.spawn(
            async move {
                let result = Self::run_internal(task, foreground.clone()).await;

                if tx.send(result).is_err() {
                    log::error!("Caller did not wait for agent driver to finish");
                }

                Self::cleanup(foreground).await;
            },
            |_, _, _| {},
        );

        async move {
            let result = match rx.await {
                Ok(result) => result,
                Err(Canceled) => {
                    log::error!("Agent driver exited abruptly");
                    Err(AgentDriverError::InvalidRuntimeState)
                }
            };

            result
        }
    }

    /// Check that the working directory exists. Since it's user-specified, we don't automatically
    /// create the directory (in case they made a typo).
    fn check_working_dir(&self) -> impl Future<Output = Result<(), AgentDriverError>> {
        let working_dir = self.working_dir.clone();
        async move {
            match async_fs::metadata(&working_dir).await {
                Ok(metadata) => {
                    if metadata.is_dir() {
                        Ok(())
                    } else {
                        Err(AgentDriverError::InvalidWorkingDirectory {
                            path: working_dir.to_owned(),
                            source: io::ErrorKind::NotADirectory.into(),
                        })
                    }
                }
                Err(err) => Err(AgentDriverError::InvalidWorkingDirectory {
                    path: working_dir.to_owned(),
                    source: err,
                }),
            }
        }
    }

    /// Runs the agent to completion.
    /// Driving the agent mostly requires main-thread UI framework updates, but using `async` and
    /// a `ModelSpawner` lets us express the high-level process linearly rather than in a
    /// series of callbacks and state machine updates.
    async fn run_internal(
        task: Task,
        foreground: ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        safe_debug!(
            safe: ("Running agent driver"),
            full: ("Running agent driver for query `{:?}`", task.prompt)
        );

        foreground
            .spawn(|me, _| me.check_working_dir())
            .await?
            .await?;

        // IMPORTANT: Wait for the terminal session to bootstrap before starting MCP servers.
        // Some of the initializations are necessary for the MCP servers to start correctly.
        //
        // Why: MCP server startup can happen before we actually execute the agent prompt. For
        // `TransportType::CLIServer` MCPs we currently depend on `AISettings.mcp_execution_path`,
        // which is populated as part of terminal bootstrap. Waiting for the session bootstrap
        // here avoids a subtle race where MCP spawn runs with an unset PATH and then the driver
        // only fails via a timeout.
        foreground
            .spawn(|me, ctx| {
                me.terminal_driver
                    .as_ref(ctx)
                    .wait_for_session_bootstrapped()
            })
            .await?
            .await?;

        // Run the harness with a prompt
        match task.harness {
            HarnessKind::ThirdParty(harness) => {
                let harness_exit_rx = Self::setup_harness(harness.as_ref(), &foreground).await?;
                let runner =
                    Self::prepare_harness(&task.prompt, harness.as_ref(), &foreground).await?;
                Self::run_harness(runner, &foreground, harness_exit_rx).await
            }
            HarnessKind::Unsupported(harness) => Err(AgentDriverError::HarnessSetupFailed {
                harness: harness.to_string(),
                reason: format!(
                    "The {harness} harness is only supported for local child agent launches."
                ),
            }),
        }
    }

    /// Sets up the third-party harness by subscribing to CLI session events and
    /// installing the Warp plugin and platform plugin, if applicable.
    ///
    /// Returns a oneshot receiver that fires when the harness should exit
    /// (either immediately on completion or after the idle-on-complete timeout).
    async fn setup_harness(
        harness: &dyn ThirdPartyHarness,
        foreground: &ModelSpawner<Self>,
    ) -> Result<oneshot::Receiver<()>, AgentDriverError> {
        let (exit_tx, exit_rx) = oneshot::channel();
        let harness_exit = IdleTimeoutSender::new(exit_tx);

        // Subscribe to CLI agent session events so we can update the task
        // state as the harness emits stop/blocked notifications.
        foreground
            .spawn(move |me, ctx| me.subscribe_to_cli_agent_session_events(harness_exit, ctx))
            .await?;

        // Install plugins before running the harness command.
        let plugin_manager: Option<Box<dyn CliAgentPluginManager>> =
            plugin_manager_for(harness.cli_agent());
        if let Some(manager) = plugin_manager {
            if let Err(e) = manager.install().await {
                log::warn!("Plugin installation failed (continuing): {e}");
            }
            if let Err(e) = manager.install_platform_plugin().await {
                log::warn!("Platform plugin installation failed (continuing): {e}");
            }
        }

        Ok(exit_rx)
    }

    /// Configure a third-party harness for execution. This will set `self.harness` and
    /// return a handle to the harness runner.
    async fn prepare_harness(
        prompt: &AgentRunPrompt,
        harness: &dyn ThirdPartyHarness,
        foreground: &ModelSpawner<Self>,
    ) -> Result<Arc<dyn harness::HarnessRunner>, AgentDriverError> {
        let (working_dir, terminal_driver) = foreground
            .spawn(|me, _ctx| {
                if me.harness.is_some() {
                    log::error!(
                        "Attempted to prepare a third-party harness, but one was already configured"
                    );
                    return Err(AgentDriverError::InvalidRuntimeState);
                }

                Ok((me.working_dir.clone(), me.terminal_driver.clone()))
            })
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)
            .flatten()?;

        let (prompt_text, system_prompt): (Cow<'_, str>, Option<String>) = match prompt {
            AgentRunPrompt::Local(text) => (Cow::Borrowed(text), None),
        };

        // Prepare harness config files for trust dialogs, API-key approval, etc.
        let secrets = foreground
            .spawn(|me, _| Arc::clone(&me.secrets))
            .await
            .map_err(|_| AgentDriverError::InvalidRuntimeState)?;
        harness.prepare_environment_config(&working_dir, system_prompt.as_deref(), &secrets)?;

        let runner: Arc<dyn HarnessRunner> = harness
            .build_runner(
                prompt_text.as_ref(),
                system_prompt.as_deref(),
                &working_dir,
                terminal_driver,
            )?
            .into();

        let stored_runner = runner.clone();
        foreground
            .spawn(move |me, _| me.harness = Some(stored_runner))
            .await?;

        Ok(runner)
    }

    /// Execute a configured external harness in the terminal.
    ///
    /// The `harness_exit_rx` oneshot fires when the subscription determines it's
    /// time to exit (either immediately on completion or after the idle timeout).
    async fn run_harness(
        runner: Arc<dyn harness::HarnessRunner>,
        foreground: &ModelSpawner<Self>,
        harness_exit_rx: oneshot::Receiver<()>,
    ) -> Result<(), AgentDriverError> {
        // Start the third-party harness.
        let mut command_handle = runner.start(foreground).await?.fuse();
        let mut harness_exit_rx = harness_exit_rx.fuse();

        // Periodically save the conversation while the command is running and handle
        // exiting gracefully once the idle timeout elapses.
        let command_result = loop {
            futures::select! {
                exit_code = command_handle => break exit_code,
                _ = warpui::r#async::Timer::after(HARNESS_SAVE_INTERVAL).fuse() => {
                    log::debug!("Triggering periodic save of harness conversation data");
                    report_if_error!(runner
                        .save_conversation(SavePoint::Periodic, foreground)
                        .await
                        .context("Failed to save harness conversation (periodic)"));
                }
                _ = harness_exit_rx => {
                    log::debug!("Requesting harness exit");
                    report_if_error!(runner
                        .exit(foreground)
                        .await
                        .context("Failed to exit harness"));
                }
            }
        };

        // Final save after the command finishes.
        log::debug!("Triggering final save of harness conversation data");
        report_if_error!(runner
            .save_conversation(SavePoint::Final, foreground)
            .await
            .context("Failed to save harness conversation (final)"));
        report_if_error!(runner
            .cleanup(foreground)
            .await
            .context("Failed to clean up harness runtime state"));

        let exit_code = command_result?;
        log::debug!("Agent harness exited with status {exit_code}");

        if exit_code.was_successful() {
            Ok(())
        } else {
            Err(AgentDriverError::HarnessCommandFailed {
                exit_code: exit_code.value(),
            })
        }
    }

    /// Subscribe to the singleton `CLIAgentSessionsModel` so that idle-on-complete
    /// timers are driven by CLI agent session status changes.
    ///
    fn subscribe_to_cli_agent_session_events(
        &self,
        harness_exit: IdleTimeoutSender<()>,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_view_id = self.terminal_driver.as_ref(ctx).terminal_view().id();

        ctx.subscribe_to_model(
            &CLIAgentSessionsModel::handle(ctx),
            move |me, event, ctx| match event {
                CLIAgentSessionsModelEvent::StatusChanged {
                    terminal_view_id: event_tid,
                    status,
                    ..
                } => {
                    if *event_tid != terminal_view_id {
                        return;
                    }

                    // Drive idle-on-complete timer for the harness exit signal.
                    match status {
                        CLIAgentSessionStatus::Success | CLIAgentSessionStatus::Blocked { .. } => {
                            if let Some(idle_timeout) = me.idle_on_complete {
                                harness_exit.end_run_after(idle_timeout, ());
                            } else {
                                harness_exit.end_run_now(());
                            }
                        }
                        CLIAgentSessionStatus::InProgress => {
                            harness_exit.cancel_idle_timeout();
                        }
                    }
                }
                CLIAgentSessionsModelEvent::SessionUpdated {
                    terminal_view_id: event_tid,
                    ..
                } => {
                    if *event_tid != terminal_view_id {
                        return;
                    }

                    let Some(runner) = me.harness.clone() else {
                        return;
                    };
                    let spawner = ctx.spawner();
                    ctx.spawn(
                        async move {
                            log::debug!(
                                "Triggering post-turn harness session update from CLI agent event"
                            );
                            report_if_error!(runner
                                .handle_session_update(&spawner)
                                .await
                                .context("Failed to update harness state from CLI session event"));
                            log::debug!("Triggering post-turn save of harness conversation data");
                            report_if_error!(runner
                                .save_conversation(SavePoint::PostTurn, &spawner)
                                .await
                                .context("Failed to save harness conversation (post-turn)"));
                        },
                        |_, _, _| {},
                    );
                }
                CLIAgentSessionsModelEvent::Started { .. }
                | CLIAgentSessionsModelEvent::InputSessionChanged { .. }
                | CLIAgentSessionsModelEvent::Ended { .. } => {}
            },
        );
    }

    /// Handle events re-emitted by the `TerminalDriver`.
    fn handle_terminal_driver_event(
        &mut self,
        event: &TerminalDriverEvent,
        _ctx: &mut ModelContext<Self>,
    ) {
        match event {
            TerminalDriverEvent::SlowBootstrap => {
                eprintln!(
                    "Warning: Terminal session is slow to bootstrap. See https://docs.warp.dev/support-and-community/troubleshooting-and-support/known-issues#shells to troubleshoot."
                );
            }
        }
    }

    /// Perform cleanup after the agent has finished running.
    async fn cleanup(_spawner: ModelSpawner<Self>) {}
}

impl Entity for AgentDriver {
    type Event = ();
}

/// The only reason that `AgentDriver` is a singleton entity is to ensure the UI framework
/// doesn't drop it. Generally, we should not assume there's only one running agent.
impl SingletonEntity for AgentDriver {}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
