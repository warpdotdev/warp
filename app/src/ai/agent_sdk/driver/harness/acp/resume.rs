//! Continuing a restored conversation's agent session in a new sandbox.
//!
//! The CLI behind an ACP adapter persists its session as the same transcript its terminal runner
//! resumes from, and the run uploads it (see [`super::cli_transcript`]). A run that continues a
//! restored conversation writes that transcript back where the CLI looks for it and asks the
//! adapter to reopen it, so the agent keeps its memory of earlier turns the way
//! `claude --resume` and `codex resume` do on the terminal transport.
//!
//! ACP has two methods for reopening a session, and what the agent announces at `initialize`
//! decides which one is sent. `session/resume` is preferred: the agent restores the session and
//! answers. An agent that only takes `session/load` replays the whole history as
//! `session/update`s before it answers; the native conversation was already restored from the
//! server, so that replay is discarded rather than applied to it a second time.
//!
//! The run opens a fresh session, as every run did before, when there is nothing to reopen: the
//! harness has no restorable transcript, the previous run never uploaded one, or the agent takes
//! neither method or refuses the one it was sent. A transcript that exists but cannot be
//! downloaded or written back fails the run instead, as it does on the terminal transport.
//!
//! Known limitation: when claude-agent-acp restarts a session after a plan is approved, the
//! transcript upload stops at the restart (see [`super::cli_transcript`]), so a resumed session
//! only carries the turns up to that point.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use futures::channel::oneshot;
use futures::future::AbortHandle;
use instant::Instant;
use parking_lot::Mutex;
use warpui::r#async::Timer;
use warpui::r#async::executor::Background;

use super::super::claude_transcript::{ClaudeTranscriptEnvelope, rehydrate_claude_transcript};
use super::super::codex_transcript::{CodexTranscriptEnvelope, rehydrate_codex_transcript};
use super::super::fetch_transcript_envelope;
use super::cli_transcript::CliTranscriptKind;
use super::connection::{AcpConnection, RpcError};
use super::protocol::{
    self, AgentCapabilities, LoadSessionParams, LoadSessionResponse, McpServer, NewSessionParams,
    NewSessionResponse, ResumeSessionParams, ResumeSessionResponse,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent_sdk::driver::{AgentDriverError, HarnessSetupStage};
use crate::server::server_api::harness_support::HarnessSupportClient;

/// The ACP method that reopens a session the agent persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReopenMethod {
    /// `session/resume`: the agent restores the session and answers.
    Resume,
    /// `session/load`: the agent replays the session's history as notifications before it
    /// answers, and the replay is discarded.
    Load,
}

impl ReopenMethod {
    /// The method an agent with these capabilities is asked to reopen a session through, or
    /// `None` when it announces neither. `session/resume` wins when both are announced, since
    /// it spares the replay.
    pub(super) fn for_agent(capabilities: &AgentCapabilities) -> Option<Self> {
        if capabilities.session_capabilities.supports_resume() {
            Some(Self::Resume)
        } else if capabilities.load_session {
            Some(Self::Load)
        } else {
            None
        }
    }

    fn wire_method(self) -> &'static str {
        match self {
            Self::Resume => protocol::METHOD_SESSION_RESUME,
            Self::Load => protocol::METHOD_SESSION_LOAD,
        }
    }
}

impl std::fmt::Display for ReopenMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.wire_method())
    }
}

/// Where the run's ACP session came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SessionOrigin {
    /// The run started a new conversation.
    New,
    /// The restored conversation's agent session was reopened through this method, so the agent
    /// remembers its earlier turns.
    Resumed(ReopenMethod),
    /// The restored conversation's agent session could not be reopened, so a fresh one was
    /// opened.
    ResumeMissed(ResumeMiss),
}

impl SessionOrigin {
    /// A bounded label for metrics.
    pub(super) fn metric_label(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Resumed(ReopenMethod::Resume) => "resumed",
            Self::Resumed(ReopenMethod::Load) => "loaded",
            Self::ResumeMissed(miss) => miss.metric_label(),
        }
    }
}

/// Why a restored conversation's agent session was not reopened. None of these fail the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResumeMiss {
    /// This client cannot restore the harness's session transcript.
    Unsupported,
    /// The directory the CLI keeps its transcripts under could not be located when the run was
    /// prepared, which `build_runner` logged at the time.
    TranscriptRootUnresolved(CliTranscriptKind),
    /// The previous run never uploaded a session transcript.
    NoTranscript,
    /// The agent announces neither `session/resume` nor `session/load`.
    AgentCannotReopen,
    /// The agent answered the reopening request with an error.
    Rejected { method: ReopenMethod, error: String },
}

impl ResumeMiss {
    /// A bounded label for metrics.
    pub(super) fn metric_label(&self) -> &'static str {
        match self {
            Self::Unsupported => "resume_missed_unsupported",
            Self::TranscriptRootUnresolved(_) => "resume_missed_transcript_root_unresolved",
            Self::NoTranscript => "resume_missed_no_transcript",
            Self::AgentCannotReopen => "resume_missed_agent_cannot_reopen",
            Self::Rejected {
                method: ReopenMethod::Resume,
                ..
            } => "resume_missed_rejected",
            Self::Rejected {
                method: ReopenMethod::Load,
                ..
            } => "resume_missed_load_rejected",
        }
    }
}

impl std::fmt::Display for ResumeMiss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => write!(f, "the harness's session transcript cannot be restored"),
            Self::TranscriptRootUnresolved(kind) => write!(
                f,
                "the {kind:?} transcript directory could not be located (see the \"Cannot locate \
                 {kind:?} transcripts\" warning logged when the run was prepared)"
            ),
            Self::NoTranscript => write!(f, "no session transcript was saved"),
            Self::AgentCannotReopen => write!(
                f,
                "the agent supports neither {} nor {}",
                ReopenMethod::Resume,
                ReopenMethod::Load
            ),
            Self::Rejected { method, error } => write!(f, "the agent refused {method}: {error}"),
        }
    }
}

/// Whether the restored conversation's transcript is one this client can put back for the CLI,
/// decided before the agent launches so the download can start alongside it. `kind` is the
/// harness's transcript kind and `root_resolved` whether its transcript directory was located.
pub(super) fn restorable_transcript(
    kind: Option<CliTranscriptKind>,
    root_resolved: bool,
) -> Result<CliTranscriptKind, ResumeMiss> {
    match kind {
        None | Some(CliTranscriptKind::Pi) => Err(ResumeMiss::Unsupported),
        Some(kind) if !root_resolved => Err(ResumeMiss::TranscriptRootUnresolved(kind)),
        Some(kind) => Ok(kind),
    }
}

/// Why the restored conversation's transcript was not put back.
#[derive(Debug)]
pub(super) enum RehydrateError {
    /// There is nothing to resume; the run continues in a new session.
    Missed(ResumeMiss),
    /// The transcript exists but could not be downloaded, read, or written back. The run fails
    /// with this error, as a terminal-transport resume would.
    Failed(AgentDriverError),
}

/// Downloads the restored conversation's CLI session transcript and writes it back where the CLI
/// reads it, returning the session id to resume.
pub(super) async fn rehydrate_cli_session(
    kind: CliTranscriptKind,
    conversation_id: &ServerConversationToken,
    client: Arc<dyn HarnessSupportClient>,
    cwd: &Path,
) -> Result<String, RehydrateError> {
    let cwd = cwd.to_path_buf();
    let written = match kind {
        CliTranscriptKind::Claude => {
            let mut envelope: ClaudeTranscriptEnvelope =
                fetch_transcript_envelope("claude", conversation_id, client)
                    .await
                    .map_err(fetch_failure)?;
            tokio::task::spawn_blocking(move || {
                rehydrate_claude_transcript(&mut envelope, &cwd).map(|_| envelope.uuid.to_string())
            })
            .await
        }
        CliTranscriptKind::Codex => {
            let mut envelope: CodexTranscriptEnvelope =
                fetch_transcript_envelope("codex", conversation_id, client)
                    .await
                    .map_err(fetch_failure)?;
            tokio::task::spawn_blocking(move || {
                rehydrate_codex_transcript(&mut envelope, &cwd)
                    .map(|_| envelope.session_id.to_string())
            })
            .await
        }
        CliTranscriptKind::Pi => return Err(RehydrateError::Missed(ResumeMiss::Unsupported)),
    };
    match written {
        Ok(Ok(session_id)) => Ok(session_id),
        Ok(Err(error)) => Err(RehydrateError::Failed(AgentDriverError::ConfigBuildFailed(
            error.context("Failed to write the session transcript back for the CLI"),
        ))),
        Err(error) => Err(RehydrateError::Failed(AgentDriverError::ConfigBuildFailed(
            anyhow!("transcript rehydration task failed: {error}"),
        ))),
    }
}

fn fetch_failure(error: AgentDriverError) -> RehydrateError {
    match error {
        AgentDriverError::ConversationResumeStateMissing { .. } => {
            RehydrateError::Missed(ResumeMiss::NoTranscript)
        }
        error => RehydrateError::Failed(error),
    }
}

/// A transcript being put back on the background executor while the agent launches and
/// initializes. Dropping it aborts the download; a write-back already under way completes.
pub(super) struct PendingRehydration {
    result: oneshot::Receiver<Result<String, RehydrateError>>,
    abort: AbortHandle,
}

impl PendingRehydration {
    pub(super) fn spawn(
        background: &Background,
        kind: CliTranscriptKind,
        conversation_id: ServerConversationToken,
        client: Arc<dyn HarnessSupportClient>,
        cwd: PathBuf,
    ) -> Self {
        let (result_tx, result) = oneshot::channel();
        let (task, abort) = background.spawn_abortable(async move {
            let _ =
                result_tx.send(rehydrate_cli_session(kind, &conversation_id, client, &cwd).await);
        });
        task.detach();
        Self { result, abort }
    }

    #[cfg(test)]
    pub(super) fn from_parts(
        result: oneshot::Receiver<Result<String, RehydrateError>>,
        abort: AbortHandle,
    ) -> Self {
        Self { result, abort }
    }

    async fn wait(mut self) -> Result<String, RehydrateError> {
        match (&mut self.result).await {
            Ok(result) => result,
            Err(oneshot::Canceled) => {
                Err(RehydrateError::Failed(AgentDriverError::ConfigBuildFailed(
                    anyhow!("transcript rehydration task was dropped before it finished"),
                )))
            }
        }
    }
}

impl Drop for PendingRehydration {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

/// How the handshake opens the run's session.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SessionPlan {
    /// Start a new session.
    New,
    /// Reopen the CLI session with this id, which has been put back on disk.
    Reopen {
        method: ReopenMethod,
        session_id: String,
    },
    /// Start a new session because the restored conversation's could not be reopened.
    Missed(ResumeMiss),
}

/// Decides the session plan once `initialize` has said what the agent supports. `rehydration` is
/// `None` when the run does not continue a restored conversation; otherwise the transcript being
/// put back, or why it is not. The download is waited for only when the agent can reopen a
/// session, and abandoned otherwise.
pub(super) async fn plan_session(
    rehydration: Option<Result<PendingRehydration, ResumeMiss>>,
    capabilities: &AgentCapabilities,
) -> Result<SessionPlan, AgentDriverError> {
    let pending = match rehydration {
        None => return Ok(SessionPlan::New),
        Some(Err(miss)) => return Ok(SessionPlan::Missed(miss)),
        Some(Ok(pending)) => pending,
    };
    let Some(method) = ReopenMethod::for_agent(capabilities) else {
        drop(pending);
        return Ok(SessionPlan::Missed(ResumeMiss::AgentCannotReopen));
    };
    match pending.wait().await {
        Ok(session_id) => Ok(SessionPlan::Reopen { method, session_id }),
        Err(RehydrateError::Missed(miss)) => Ok(SessionPlan::Missed(miss)),
        Err(RehydrateError::Failed(error)) => Err(error),
    }
}

/// The handshake's deadline, counted from the start of its current phase. Restarting it at a
/// phase boundary gives the next phase the full budget, so a `session/new` that follows a slow,
/// refused reopening is not cut short by the time the refusal took.
#[derive(Clone)]
pub(super) struct PhaseClock(Arc<Mutex<Phase>>);

struct Phase {
    started: Instant,
    stage: HarnessSetupStage,
    name: &'static str,
}

impl PhaseClock {
    pub(super) fn start(stage: HarnessSetupStage, name: &'static str) -> Self {
        Self(Arc::new(Mutex::new(Phase {
            started: Instant::now(),
            stage,
            name,
        })))
    }

    pub(super) fn restart(&self, stage: HarnessSetupStage, name: &'static str) {
        *self.0.lock() = Phase {
            started: Instant::now(),
            stage,
            name,
        };
    }

    /// What the handshake is doing now, for the timeout's error.
    pub(super) fn phase(&self) -> &'static str {
        self.0.lock().name
    }

    /// The setup stage the handshake is in now, for the failure's classification.
    pub(super) fn stage(&self) -> HarnessSetupStage {
        self.0.lock().stage
    }

    /// Resolves once the current phase has run for `budget` without a restart.
    pub(super) async fn expire(&self, budget: Duration) {
        loop {
            let remaining = budget.saturating_sub(self.0.lock().started.elapsed());
            if remaining.is_zero() {
                return;
            }
            Timer::after(remaining).await;
        }
    }
}

/// The phase a refused reopening moves the handshake into.
pub(super) const FALLBACK_SESSION_PHASE: &str =
    "opening a new session after refusing to reopen its earlier one";

/// Opens the run's ACP session according to `plan`. A reopening the agent answers with an error
/// falls back to `session/new` with a fresh phase on `clock`; a request that fails for any other
/// reason (the agent went away, the connection broke) is the handshake's error, since the same
/// connection cannot open a session either. What a `session/load` replays is discarded whether
/// the agent then answers with the session or with an error.
pub(super) async fn open_session(
    connection: &AcpConnection,
    plan: SessionPlan,
    cwd: String,
    mcp_servers: Vec<McpServer>,
    clock: &PhaseClock,
) -> Result<(NewSessionResponse, SessionOrigin)> {
    let origin = match plan {
        SessionPlan::New => SessionOrigin::New,
        SessionPlan::Missed(miss) => SessionOrigin::ResumeMissed(miss),
        SessionPlan::Reopen { method, session_id } => {
            let reopened = match method {
                ReopenMethod::Resume => {
                    connection
                        .request::<_, Option<ResumeSessionResponse>>(
                            protocol::METHOD_SESSION_RESUME,
                            ResumeSessionParams {
                                session_id: session_id.clone(),
                                cwd: cwd.clone(),
                                mcp_servers: mcp_servers.clone(),
                            },
                        )
                        .await
                }
                ReopenMethod::Load => {
                    connection
                        .request_discarding_replay::<_, Option<LoadSessionResponse>>(
                            protocol::METHOD_SESSION_LOAD,
                            LoadSessionParams {
                                session_id: session_id.clone(),
                                cwd: cwd.clone(),
                                mcp_servers: mcp_servers.clone(),
                            },
                        )
                        .await
                }
            };
            match reopened {
                Ok(reopened) => {
                    let session = reopened.unwrap_or_default().into_session(session_id);
                    return Ok((session, SessionOrigin::Resumed(method)));
                }
                Err(error) => match error.downcast::<RpcError>() {
                    Ok(refusal) => {
                        clock.restart(HarnessSetupStage::SessionOpen, FALLBACK_SESSION_PHASE);
                        SessionOrigin::ResumeMissed(ResumeMiss::Rejected {
                            method,
                            error: refusal.to_string(),
                        })
                    }
                    Err(error) => return Err(error.context(format!("{method} failed"))),
                },
            }
        }
    };
    let session = connection
        .request(
            protocol::METHOD_SESSION_NEW,
            NewSessionParams { cwd, mcp_servers },
        )
        .await
        .context("session/new failed")?;
    Ok((session, origin))
}

#[cfg(test)]
#[path = "resume_tests.rs"]
mod tests;
