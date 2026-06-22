use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::channel::oneshot;
use session_sharing_protocol::common::{Role, SessionId};
use session_sharing_protocol::sharer::SessionRetentionReason;
use warp_cli::share::{ShareAccessLevel, ShareRequest, ShareSubject};
use warp_completer::completer::CommandOutput;
use warp_core::command::ExitCode;
use warp_core::features::FeatureFlag;
use warp_terminal::model::grid::Dimensions;
use warp_util::path::ShellFamily;
use warpui::r#async::FutureExt;
use warpui::{AppContext, Entity, ModelContext, ModelHandle, SingletonEntity as _, ViewHandle};

use super::AgentDriverError;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::attachment_utils::attachments_download_dir;
use crate::pane_group::NewTerminalOptions;
use crate::root_view::{open_new_with_workspace_source, NewWorkspaceSource};
use crate::terminal::model::block::{BlockId, SerializedBlock};
use crate::terminal::model::find::RegexDFAs;
use crate::terminal::model::grid::RespectDisplayedOutput;
use crate::terminal::model::index::Point;
use crate::terminal::model::session::ExecuteCommandOptions;
use crate::terminal::model::RespectObfuscatedSecrets;
use crate::terminal::shared_session::{self, IsSharedSessionCreator, SharedSessionSource};
use crate::terminal::shell::ShellType;
use crate::terminal::view::{ConversationRestorationInNewPaneType, Event};
use crate::terminal::TerminalView;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Describes why a terminal session bootstrap failed.
#[derive(Debug)]
pub(crate) enum BootstrapError {
    /// The PTY or shell process failed before the bootstrap script completed.
    /// When `reason` is `Some`, the message is
    /// "Shell spawn failed: {reason}. Check the Warp logs for details."
    /// When `reason` is `None`, it is
    /// "Shell spawn failed. Check the Warp logs for details."
    PtySpawnFailed { reason: Option<String> },
    /// The bootstrap script did not complete within the expected time.
    TimedOut,
    /// An unexpected internal error in the bootstrap channel
    /// (e.g. the sender was dropped without sending).
    InternalError,
}

impl std::fmt::Display for BootstrapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootstrapError::PtySpawnFailed { reason: Some(r) } => {
                write!(
                    f,
                    "Shell spawn failed: {r}. Check the Warp logs for details."
                )
            }
            BootstrapError::PtySpawnFailed { reason: None } => {
                write!(f, "Shell spawn failed. Check the Warp logs for details.")
            }
            BootstrapError::TimedOut => write!(
                f,
                "Terminal session did not start within the expected time. \
                 Check the Warp logs for details."
            ),
            BootstrapError::InternalError => {
                write!(f, "An unexpected internal error occurred during bootstrap.")
            }
        }
    }
}

impl std::error::Error for BootstrapError {}

/// Describes why an agent's session-sharing request failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ShareSessionError {
    /// Connection to the session-sharing server failed.
    #[error("Internal error")]
    Internal(#[source] Arc<anyhow::Error>),
    /// The server rejected the session-sharing request.
    #[error("{0}")]
    Failed(String),
    /// Session sharing is disabled for this user or team.
    #[error(
        "Session sharing is not enabled. This is likely because an administrator has disabled session sharing for your team."
    )]
    Disabled,
    /// The session-sharing request timed out.
    #[error("Timed out waiting for session sharing to start")]
    Timeout,
    /// The session-sharing channel was dropped before completing.
    #[error("Session sharing was interrupted")]
    Interrupted,
}

const TERMINAL_SESSION_BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(60);
/// The total time to wait for session sharing to start, including retries.
const TERMINAL_SESSION_SHARE_DELAY: Duration = Duration::from_secs(20);

/// Options for creating the terminal view before constructing a [`TerminalDriver`].
pub(crate) struct TerminalDriverOptions {
    pub working_dir: PathBuf,
    pub env_vars: HashMap<OsString, OsString>,
    pub should_share: bool,
    pub task_id: Option<AmbientAgentTaskId>,
    pub conversation_restoration: Option<ConversationRestorationInNewPaneType>,
}

/// Events emitted by [`TerminalDriver`] for [`super::AgentDriver`] to react to.
pub(crate) enum TerminalDriverEvent {
    /// Terminal bootstrap is taking unusually long.
    SlowBootstrap,
    /// The terminal session has established a shared session.
    EstablishedSharedSession {
        session_id: session_sharing_protocol::common::SessionId,
        join_url: String,
    },
}

/// Manages the terminal session lifecycle for the agent driver.
///
/// Responsibilities:
/// - Monitoring for terminal bootstrapping to be done
/// - Configuring session sharing and applying guest requests
/// - Executing commands in the session
/// - Detecting block completion
pub(crate) struct TerminalDriver {
    terminal_view: ViewHandle<TerminalView>,
    /// Sender half of the bootstrap result channel. Exactly one of
    /// `SessionBootstrapped`, `PtySpawnFailed`, or `Exited` (pre-bootstrap)
    /// will send the outcome; all others no-op after the first send.
    bootstrap_tx: Option<oneshot::Sender<Result<(), BootstrapError>>>,
    /// Receiver half consumed by `wait_for_session_bootstrapped`.
    bootstrap_rx: Option<oneshot::Receiver<Result<(), BootstrapError>>>,
    /// The session ID once sharing has been established.
    shared_session_id: Option<SessionId>,
    /// Receiver for the session sharing result. Present when sharing is expected
    /// and `wait_for_session_shared` has not yet been called.
    session_share_rx: Option<oneshot::Receiver<Result<(), ShareSessionError>>>,
    pending_share_requests: Vec<ShareRequest>,
    waiting_command: Option<oneshot::Sender<ExitCode>>,

    /// State for the pending command we're expecting to start executing.
    /// The `String` is the expected command text, and the sender is used
    /// to send the block ID to the waiting caller.
    pending_command_start: Option<(String, oneshot::Sender<BlockId>)>,
}

impl Entity for TerminalDriver {
    type Event = TerminalDriverEvent;
}

/// Create the terminal window and extract the [`ViewHandle<TerminalView>`].
///
/// This is separate from [`TerminalDriver::new`] because [`AppContext::add_model`]
/// requires an infallible constructor; the fallible window/view creation must happen first.
fn create_terminal_view(
    options: TerminalDriverOptions,
    ctx: &mut AppContext,
) -> Result<ViewHandle<TerminalView>, AgentDriverError> {
    let is_shared_session_creator = if options.should_share {
        IsSharedSessionCreator::Yes {
            source: SharedSessionSource::ambient_agent(options.task_id.map(|t| t.to_string())),
        }
    } else {
        IsSharedSessionCreator::No
    };

    let (_, root_view) = open_new_with_workspace_source(
        NewWorkspaceSource::Session {
            options: Box::new(NewTerminalOptions {
                is_shared_session_creator,
                initial_directory: Some(options.working_dir),
                env_vars: options.env_vars,
                conversation_restoration: options.conversation_restoration,
                ..Default::default()
            }),
        },
        ctx,
    );

    root_view
        .as_ref(ctx)
        .workspace_view()
        .ok_or(AgentDriverError::TerminalUnavailable)?
        .as_ref(ctx)
        .active_tab_pane_group()
        .as_ref(ctx)
        .active_session_view(ctx)
        .ok_or(AgentDriverError::TerminalUnavailable)
}

impl TerminalDriver {
    /// Create a terminal view from the given options and wrap it in a new `TerminalDriver` model.
    pub(crate) fn create(
        options: TerminalDriverOptions,
        ctx: &mut AppContext,
    ) -> Result<ModelHandle<Self>, AgentDriverError> {
        let should_share = options.should_share;
        let task_id = options.task_id;
        let working_dir = options.working_dir.clone();
        let terminal_view = create_terminal_view(options, ctx)?;
        Ok(ctx.add_model(|ctx| Self::new(terminal_view, should_share, task_id, working_dir, ctx)))
    }

    /// Wrap an already-created terminal view in a new `TerminalDriver` model.
    ///
    /// Unlike [`Self::create`], this does not open a new window — it reuses an
    /// existing view (e.g. a docker sandbox pane). Session sharing is disabled
    /// and no task ID is associated.
    pub(crate) fn create_from_existing_view(
        terminal_view: ViewHandle<TerminalView>,
        ctx: &mut AppContext,
    ) -> ModelHandle<Self> {
        ctx.add_model(|ctx| Self::new(terminal_view, false, None, PathBuf::default(), ctx))
    }

    /// Set up event subscriptions and session-sharing conditions for an
    /// already-created terminal view.
    fn new(
        terminal_view: ViewHandle<TerminalView>,
        should_share: bool,
        task_id: Option<AmbientAgentTaskId>,
        working_dir: PathBuf,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        // Create a oneshot channel for session sharing when sharing is expected.
        // When sharing is disabled (or running against ngrok), leave both halves
        // as None so that `wait_for_session_shared` returns immediately.
        let sharing_expected =
            should_share && !warp_core::channel::ChannelState::server_root_url().contains("ngrok");
        let (mut session_share_tx, session_share_rx) = if sharing_expected {
            if !FeatureFlag::CreatingSharedSessions.is_enabled() {
                // Session sharing was requested but the feature is not enabled for this
                // user/team (typically an enterprise/admin setting). Fail immediately
                // with a clear error rather than waiting for a timeout.
                log::warn!(
                    "Session sharing requested but the CreatingSharedSessions feature flag \
                     is not enabled. This is likely due to a team administrator disabling \
                     session sharing."
                );
                let (tx, rx) = oneshot::channel();
                let _ = tx.send(Err(ShareSessionError::Disabled));
                (None, Some(rx))
            } else {
                log::info!("Waiting for requested session sharing to start");
                let (tx, rx) = oneshot::channel();
                (Some(tx), Some(rx))
            }
        } else {
            (None, None)
        };

        // Set the task_id and attachments download dir on the AI controller right away
        // so they're available for session sharing and file downloads.
        // Only set the download dir when a task_id is present (cloud mode),
        // since attachments require a task to fetch presigned URLs.
        if let Some(tid) = task_id {
            let attachments_dir = attachments_download_dir(&working_dir);
            terminal_view.update(ctx, |terminal, ctx| {
                terminal.ai_controller().update(ctx, |controller, ctx| {
                    controller.set_ambient_agent_task_id(Some(tid), ctx);
                    controller.set_attachments_download_dir(attachments_dir);
                });
            });
        }

        ctx.subscribe_to_view(&terminal_view, move |me, _, event, ctx| {
            me.handle_terminal_view_event(event, &mut session_share_tx, ctx);
        });

        let (bootstrap_tx_inner, bootstrap_rx) = oneshot::channel::<Result<(), BootstrapError>>();

        // If bootstrap already completed before we subscribed, resolve the
        // channel immediately so `wait_for_session_bootstrapped` doesn't block.
        let already_bootstrapped = terminal_view.read(ctx, |terminal, _| {
            terminal
                .model
                .lock()
                .block_list()
                .is_bootstrapping_precmd_done()
        });
        let bootstrap_tx = if already_bootstrapped {
            let _ = bootstrap_tx_inner.send(Ok(()));
            None
        } else {
            Some(bootstrap_tx_inner)
        };

        Self {
            terminal_view,
            bootstrap_tx,
            bootstrap_rx: Some(bootstrap_rx),
            shared_session_id: None,
            session_share_rx,
            pending_share_requests: Vec::new(),
            waiting_command: None,
            pending_command_start: None,
        }
    }

    /// Get a handle to the backing terminal view.
    pub fn terminal_view(&self) -> &ViewHandle<TerminalView> {
        &self.terminal_view
    }

    /// Provide mutable access to the terminal view through a closure.
    pub fn with_terminal_view(
        &self,
        ctx: &mut ModelContext<Self>,
        f: impl FnOnce(&mut TerminalView, &mut warpui::ViewContext<TerminalView>),
    ) {
        self.terminal_view.update(ctx, f);
    }

    /// Request that the terminal session be shared with the given participants.
    ///
    /// This has no effect if the session is not being shared.
    pub fn add_share_requests(
        &mut self,
        share_requests: impl IntoIterator<Item = ShareRequest>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.pending_share_requests.extend(share_requests);
        if self.shared_session_id.is_some() {
            self.apply_share_requests(ctx);
        }
    }

    /// Apply pending session-sharing guest requests.
    fn apply_share_requests(&mut self, ctx: &mut ModelContext<Self>) {
        if self.pending_share_requests.is_empty() {
            return;
        }

        let share_requests = std::mem::take(&mut self.pending_share_requests);
        self.terminal_view.update(ctx, |terminal_view, ctx| {
            let mut viewer_emails = Vec::new();
            let mut editor_emails = Vec::new();

            for request in share_requests {
                let role = match request.access_level {
                    ShareAccessLevel::View => Role::Reader,
                    ShareAccessLevel::Edit => Role::Executor,
                };

                match request.subject {
                    ShareSubject::Team => {
                        if let Some(team_uid) = UserWorkspaces::as_ref(ctx).current_team_uid() {
                            terminal_view.update_session_team_permissions(
                                Some(role),
                                team_uid.to_string(),
                                ctx,
                            );
                        }
                    }
                    ShareSubject::Public => {
                        // Apply an anyone-with-link ACL at the requested role.
                        // This uses the same path as the share modal's
                        // "anyone with link" toggle. The workspace-level
                        // anyone-with-link setting on the server still gates
                        // whether the ACL write succeeds.
                        terminal_view.update_session_link_permissions(Some(role), ctx);
                    }
                    ShareSubject::User { email } => match request.access_level {
                        ShareAccessLevel::View => viewer_emails.push(email),
                        ShareAccessLevel::Edit => editor_emails.push(email),
                    },
                }
            }

            if !viewer_emails.is_empty() {
                terminal_view.add_guests(viewer_emails, Role::Reader, ctx);
            }
            if !editor_emails.is_empty() {
                terminal_view.add_guests(editor_emails, Role::Executor, ctx);
            }
        });
    }

    /// Submit `text` to the active CLI agent on the terminal PTY using the
    /// agent-specific submission strategy.
    ///
    /// Used to send exit commands to third-party harnesses.
    pub(super) fn send_text_to_cli(&self, text: String, ctx: &mut ModelContext<Self>) {
        self.terminal_view.update(ctx, |terminal, ctx| {
            terminal.submit_text_to_cli_agent_pty(text, ctx);
        });
    }

    /// Return a snapshot of the block with the given ID.
    pub fn block_snapshot(&self, block_id: &BlockId, ctx: &AppContext) -> Option<SerializedBlock> {
        let terminal = self.terminal_view.as_ref(ctx);
        let model = terminal.model.lock();
        model
            .block_list()
            .block_with_id(block_id)
            .map(SerializedBlock::from)
    }

    /// Full visible plaintext of `block_id`'s output grid (no ANSI escape
    /// sequences; secrets obfuscated). Used by the harness output monitor
    /// to detect whether the block has stalled — two byte-identical
    /// snapshots taken N seconds apart imply the harness has produced no
    /// new output and no spinner activity.
    ///
    /// We intentionally pass `None` for `max_rows` so we compare the entire
    /// visible output; capping the row count could falsely report "stalled"
    /// when content scrolled below the cap actually changed.
    pub fn block_output_plaintext(&self, block_id: &BlockId, ctx: &AppContext) -> Option<String> {
        let terminal = self.terminal_view.as_ref(ctx);
        let model = terminal.model.lock();
        let block = model.block_list().block_with_id(block_id)?;
        Some(block.output_grid().contents_to_string(
            false, // include_escape_sequences
            None,  // max_rows: full visible output
        ))
    }

    pub fn find_first_match_in_block_output(
        &self,
        block_id: &BlockId,
        dfas: &RegexDFAs,
        ctx: &AppContext,
    ) -> Option<BlockOutputMatch> {
        let terminal = self.terminal_view.as_ref(ctx);
        let model = terminal.model.lock();
        let block = model.block_list().block_with_id(block_id)?;
        let grid = block.output_grid();
        let m = grid.find(dfas).next()?;
        let handler = grid.grid_handler();
        let matched_text = handler.bounds_to_string(
            *m.start(),
            *m.end(),
            false, // include_esc_sequences
            RespectObfuscatedSecrets::Yes,
            false, // force_secrets_obfuscated
            RespectDisplayedOutput::Yes,
        );
        let cols = handler.columns();
        let row_start = Point::new(m.start().row, 0);
        let row_end = Point::new(m.end().row, cols.saturating_sub(1));
        let excerpt = handler.bounds_to_string(
            row_start,
            row_end,
            false,
            RespectObfuscatedSecrets::Yes,
            false,
            RespectDisplayedOutput::Yes,
        );
        Some(BlockOutputMatch {
            matched_text: matched_text.trim().to_owned(),
            excerpt: excerpt.trim().to_owned(),
        })
    }

    /// Execute a command in the terminal and return a future that resolves to a
    /// [`CommandHandle`] once the command starts executing.
    pub fn execute_command(
        &mut self,
        command: &str,
        ctx: &mut ModelContext<Self>,
    ) -> Result<impl Future<Output = Result<CommandHandle, AgentDriverError>>, AgentDriverError>
    {
        let (exit_tx, exit_rx) = oneshot::channel::<ExitCode>();
        let (start_tx, start_rx) = oneshot::channel::<BlockId>();

        // We should not be able to execute a command while we are still waiting on another one.
        // This is enforced by the caller by waiting on rx before continuing.
        if self.waiting_command.is_some() || self.pending_command_start.is_some() {
            return Err(AgentDriverError::InvalidRuntimeState);
        }

        let command_string = command.to_string();
        self.terminal_view.update(ctx, |terminal, ctx| {
            self.waiting_command = Some(exit_tx);
            self.pending_command_start = Some((command_string, start_tx));
            terminal.execute_command_or_set_pending(command, ctx);
        });

        Ok(async move {
            let block_id = start_rx
                .await
                .map_err(|_| AgentDriverError::InvalidRuntimeState)?;
            Ok(CommandHandle {
                exit_status_rx: exit_rx,
                block_id,
            })
        })
    }

    /// Execute a command through the active session's in-band command
    /// executor, without adding a block to the user-visible blocklist.
    ///
    /// Intended for silent probes (e.g. `test -d`) that the agent needs to
    /// drive through the terminal session (so they run against the correct
    /// filesystem, including inside a Docker sandbox) but should not clutter
    /// the user's command history.
    pub fn execute_silent_command(
        &self,
        command: String,
        ctx: &ModelContext<Self>,
    ) -> impl Future<Output = Result<CommandOutput, AgentDriverError>> {
        let session = self.terminal_view.read(ctx, |terminal, app| {
            terminal
                .active_block_session_id()
                .and_then(|id| terminal.sessions_model().as_ref(app).get(id))
        });
        async move {
            let session = session.ok_or(AgentDriverError::InvalidRuntimeState)?;
            session
                .execute_command(&command, None, None, ExecuteCommandOptions::default())
                .await
                .map_err(|e| {
                    log::warn!("silent command failed: {e:#}");
                    AgentDriverError::InvalidRuntimeState
                })
        }
    }

    /// Returns the shell type of the active terminal session, if known.
    pub fn active_session_shell_type(&self, ctx: &AppContext) -> Option<ShellType> {
        self.terminal_view
            .read(ctx, |terminal, app| terminal.active_session_shell_type(app))
    }

    /// Build the shell-aware `cd <escaped>` command for the active session.
    ///
    /// Shared between [`Self::cd`] and [`Self::cd_silent`] so both paths use
    /// the same [`ShellFamily::shell_escape`] logic (posix single-quoting,
    /// fish backslash, pwsh double-quote doubling) and don't drift.
    fn build_cd_command(&self, target: &str, ctx: &AppContext) -> String {
        let shell_family = self.terminal_view.read(ctx, |terminal, app| {
            terminal
                .active_session_shell_type(app)
                .map(ShellFamily::from)
                .unwrap_or(ShellFamily::Posix)
        });
        let escaped_target = shell_family.shell_escape(target);
        format!("cd {escaped_target}")
    }

    /// Change directory within the active terminal session.
    pub fn cd(
        &mut self,
        target: &str,
        ctx: &mut ModelContext<Self>,
    ) -> Result<impl Future<Output = Result<CommandHandle, AgentDriverError>>, AgentDriverError>
    {
        let cd_command = self.build_cd_command(target, ctx);
        self.execute_command(&cd_command, ctx)
    }

    /// Change directory within the active terminal session, silently — no
    /// visible block is added to the user-facing blocklist.
    ///
    /// Uses the same shell-aware escaping as [`Self::cd`] but dispatches
    /// through [`Self::execute_silent_command`]. Intended for callers that
    /// need to position the session's CWD as an implementation detail of a
    /// larger setup step (e.g. positioning the session before running
    /// silent probes).
    pub fn cd_silent(
        &self,
        target: &str,
        ctx: &ModelContext<Self>,
    ) -> impl Future<Output = Result<CommandOutput, AgentDriverError>> {
        let cd_command = self.build_cd_command(target, ctx);
        self.execute_silent_command(cd_command, ctx)
    }

    /// The current working directory of the active terminal session, if known.
    #[allow(dead_code)]
    pub fn current_directory(&self, ctx: &AppContext) -> Option<PathBuf> {
        // TODO(ben): This should handle non-local paths.
        self.terminal_view
            .as_ref(ctx)
            .active_session_path_if_local(ctx)
    }

    /// Returns a future that resolves when the session has bootstrapped.
    ///
    /// The bootstrap result channel carries `Ok(())` on success or
    /// `Err(BootstrapError)` on failure. If the channel times out, the error
    /// is `BootstrapError::TimedOut`.
    pub fn wait_for_session_bootstrapped(
        &mut self,
    ) -> impl Future<Output = Result<(), BootstrapError>> {
        let bootstrap_rx = self.bootstrap_rx.take();

        async move {
            let result = if let Some(rx) = bootstrap_rx {
                // Map channel cancellation (sender dropped without sending)
                // to InternalError — this shouldn't happen in practice.
                let inner = async move { rx.await.unwrap_or(Err(BootstrapError::InternalError)) };
                match inner.with_timeout(TERMINAL_SESSION_BOOTSTRAP_TIMEOUT).await {
                    Ok(result) => result,
                    Err(_timeout) => Err(BootstrapError::TimedOut),
                }
            } else {
                // bootstrap_rx already consumed — shouldn't happen in normal flow.
                Err(BootstrapError::InternalError)
            };

            if let Err(ref e) = result {
                log::error!("Terminal bootstrap failed: {e}");
            }
            result
        }
    }

    /// Returns a future that resolves when (optional) session sharing has started.
    ///
    /// This is separate from `wait_for_session_bootstrapped` so that callers can:
    /// - wait for terminal bootstrap early (e.g. before starting MCP servers)
    /// - wait for session sharing later (e.g. right before running visible commands)
    pub fn wait_for_session_shared(
        &mut self,
    ) -> impl Future<Output = Result<(), AgentDriverError>> {
        let rx = self.session_share_rx.take();

        async move {
            let Some(rx) = rx else {
                // Sharing is disabled or already resolved.
                return Ok(());
            };

            match rx.with_timeout(TERMINAL_SESSION_SHARE_DELAY).await {
                Ok(Ok(Ok(()))) => Ok(()),
                Ok(Ok(Err(error))) => {
                    log::error!("Session sharing failed: {error}");
                    Err(AgentDriverError::ShareSessionFailed { error })
                }
                Ok(Err(_canceled)) => {
                    log::error!("Session sharing channel dropped");
                    Err(AgentDriverError::ShareSessionFailed {
                        error: ShareSessionError::Interrupted,
                    })
                }
                Err(_timeout) => {
                    log::error!(
                        "Timed out waiting for session sharing to start after {}s",
                        TERMINAL_SESSION_SHARE_DELAY.as_secs()
                    );
                    Err(AgentDriverError::ShareSessionFailed {
                        error: ShareSessionError::Timeout,
                    })
                }
            }
        }
    }

    pub fn extend_shared_session_retention(
        &mut self,
        reason: SessionRetentionReason,
        ctx: &mut ModelContext<Self>,
    ) {
        self.terminal_view.update(ctx, |terminal, ctx| {
            if !terminal
                .model
                .lock()
                .shared_session_status()
                .is_active_sharer()
            {
                log::warn!(
                    "Tried to extend shared session retention before sharing was active: {reason:?}"
                );
                return;
            }

            log::info!("Emitting request to extend shared session retention: {reason:?}");
            ctx.emit(Event::ExtendSessionRetention { reason });
        });
    }
}

/// The first DFA match returned by
/// [`TerminalDriver::find_first_match_in_block_output`].
///
/// `matched_text` is the exact substring from the grid (no ANSI escapes),
/// used by the harness output monitor to map the hit back to the originating
/// pattern. `excerpt` is the full row(s) containing the match, also as
/// plaintext, suitable for surfacing in user-visible error messages.
#[derive(Debug, Clone)]
pub(crate) struct BlockOutputMatch {
    pub matched_text: String,
    pub excerpt: String,
}

/// A handle to a running terminal command.
///
/// Resolves to the command's [`ExitCode`] when the block completes.
/// Also carries the [`BlockId`] so callers can retrieve the block snapshot
/// after completion.
pub(crate) struct CommandHandle {
    exit_status_rx: oneshot::Receiver<ExitCode>,
    block_id: BlockId,
}

impl CommandHandle {
    /// The block ID of the command that was executed.
    pub fn block_id(&self) -> &BlockId {
        &self.block_id
    }
}

impl Future for CommandHandle {
    type Output = Result<ExitCode, AgentDriverError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.exit_status_rx)
            .poll(cx)
            .map(|result| result.map_err(|_| AgentDriverError::InvalidRuntimeState))
    }
}

impl TerminalDriver {
    /// Handle terminal view events.
    fn handle_terminal_view_event(
        &mut self,
        event: &crate::terminal::view::Event,
        session_share_tx: &mut Option<oneshot::Sender<Result<(), ShareSessionError>>>,
        ctx: &mut ModelContext<Self>,
    ) {
        match event {
            crate::terminal::view::Event::SessionBootstrapped => {
                if let Some(tx) = self.bootstrap_tx.take() {
                    let _ = tx.send(Ok(()));
                }
            }
            crate::terminal::view::Event::PtySpawnFailed { reason } => {
                // Signal the bootstrap waiter immediately so it doesn't wait
                // for the full 60 s timeout when a spawn failure has already
                // been confirmed.
                if let Some(tx) = self.bootstrap_tx.take() {
                    let _ = tx.send(Err(BootstrapError::PtySpawnFailed {
                        reason: Some(reason.clone()),
                    }));
                }
            }
            crate::terminal::view::Event::Exited => {
                // The shell process exited before bootstrap completed —
                // cancel the wait immediately rather than sitting out the
                // full 60 s timeout. No specific reason is known at this
                // point; the logs will have details.
                if let Some(tx) = self.bootstrap_tx.take() {
                    let _ = tx.send(Err(BootstrapError::PtySpawnFailed { reason: None }));
                }
            }
            crate::terminal::view::Event::SlowBootstrap => {
                ctx.emit(TerminalDriverEvent::SlowBootstrap);
            }
            crate::terminal::view::Event::EstablishedSharedSession { session_id } => {
                self.shared_session_id = Some(*session_id);
                if let Some(tx) = session_share_tx.take() {
                    let _ = tx.send(Ok(()));
                }

                // Apply any pending share requests now that the session is established.
                self.apply_share_requests(ctx);

                ctx.emit(TerminalDriverEvent::EstablishedSharedSession {
                    session_id: *session_id,
                    join_url: shared_session::join_link(session_id),
                });
            }
            crate::terminal::view::Event::FailedToShareSession { reason, cause } => {
                if let Some(tx) = session_share_tx.take() {
                    let error = match cause {
                        Some(cause) => ShareSessionError::Internal(cause.clone()),
                        None => ShareSessionError::Failed(reason.clone()),
                    };
                    let _ = tx.send(Err(error));
                }
            }
            crate::terminal::view::Event::ExecuteCommand(event) => {
                if let Some((_expected_command, sender)) = self
                    .pending_command_start
                    .take_if(|(cmd, _)| *cmd == event.command)
                {
                    let block_id = self.terminal_view.read(ctx, |terminal, _| {
                        terminal.model.lock().block_list().active_block_id().clone()
                    });
                    let _ = sender.send(block_id);
                }
            }
            crate::terminal::view::Event::BlockCompleted { block, .. } => {
                if let Some(sender) = self.waiting_command.take_if(|_| {
                    let bootstrapping_done = self.terminal_view.read(ctx, |terminal, _| {
                        terminal
                            .model
                            .lock()
                            .block_list()
                            .is_bootstrapping_precmd_done()
                    });
                    // This was originally checking `bootstrapping_done && block.did_execute`.
                    // Oddly, we've seen cases where we missed the preexec hook, so
                    // `block.did_execute` is false even though the command actually did run.
                    // To hedge against this while we're still figuring out the root cause,
                    // we instead simply make sure it was not a background block.
                    bootstrapping_done && !block.is_background
                }) {
                    let _ = sender.send(block.exit_code);
                }
            }
            _ => (),
        }
    }
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
