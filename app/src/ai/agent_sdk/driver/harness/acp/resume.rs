//! Continuing a restored conversation's agent session in a new sandbox.
//!
//! The CLI behind an ACP adapter persists its session as the same transcript its terminal runner
//! resumes from, and the run uploads it (see [`super::cli_transcript`]). A run that continues a
//! restored conversation writes that transcript back where the CLI looks for it and asks the
//! adapter to `session/resume` it, so the agent keeps its memory of earlier turns the way
//! `claude --resume` and `codex resume` do on the terminal transport. `session/load` is not used:
//! it replays the whole history as `session/update`s, which would be applied to the restored
//! native conversation a second time.
//!
//! When the session cannot be resumed the run opens a fresh one, as every run did before.
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};

use super::super::claude_transcript::{ClaudeTranscriptEnvelope, rehydrate_claude_transcript};
use super::super::codex_transcript::{CodexTranscriptEnvelope, rehydrate_codex_transcript};
use super::super::fetch_transcript_envelope;
use super::cli_transcript::CliTranscriptKind;
use super::connection::AcpConnection;
use super::protocol::{
    self, McpServer, NewSessionParams, NewSessionResponse, ResumeSessionParams,
    ResumeSessionResponse, SessionCapabilities,
};
use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::server::server_api::harness_support::HarnessSupportClient;

/// Where the run's ACP session came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SessionOrigin {
    /// The run started a new conversation.
    New,
    /// The restored conversation's agent session was resumed.
    Resumed,
    /// The restored conversation's agent session could not be resumed, so a fresh one was opened.
    ResumeMissed(ResumeMiss),
}

/// Why a restored conversation's agent session was not resumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ResumeMiss {
    /// This client cannot restore the harness's session transcript.
    Unsupported,
    /// The previous run never uploaded a session transcript.
    NoTranscript,
    /// The transcript could not be downloaded, read, or written back.
    Rehydration(String),
    /// The agent does not announce `session/resume`.
    AgentCannotResume,
    /// The agent refused `session/resume`.
    Rejected(String),
}

impl std::fmt::Display for ResumeMiss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => write!(f, "the harness's session transcript cannot be restored"),
            Self::NoTranscript => write!(f, "no session transcript was saved"),
            Self::Rehydration(error) => write!(f, "the session transcript was unusable: {error}"),
            Self::AgentCannotResume => write!(f, "the agent does not support session/resume"),
            Self::Rejected(error) => write!(f, "the agent refused session/resume: {error}"),
        }
    }
}

/// Downloads the restored conversation's CLI session transcript and writes it back where the CLI
/// reads it, returning the session id to resume.
pub(super) async fn rehydrate_cli_session(
    kind: Option<CliTranscriptKind>,
    conversation_id: &ServerConversationToken,
    client: Arc<dyn HarnessSupportClient>,
    cwd: &Path,
) -> Result<String, ResumeMiss> {
    let cwd = cwd.to_path_buf();
    let rehydrated = match kind {
        Some(CliTranscriptKind::Claude) => {
            let mut envelope: ClaudeTranscriptEnvelope =
                fetch_transcript_envelope("claude", conversation_id, client)
                    .await
                    .map_err(miss_from_fetch)?;
            tokio::task::spawn_blocking(move || {
                rehydrate_claude_transcript(&mut envelope, &cwd).map(|_| envelope.uuid)
            })
            .await
        }
        Some(CliTranscriptKind::Codex) => {
            let mut envelope: CodexTranscriptEnvelope =
                fetch_transcript_envelope("codex", conversation_id, client)
                    .await
                    .map_err(miss_from_fetch)?;
            tokio::task::spawn_blocking(move || {
                rehydrate_codex_transcript(&mut envelope, &cwd).map(|_| envelope.session_id)
            })
            .await
        }
        Some(CliTranscriptKind::Pi) | None => return Err(ResumeMiss::Unsupported),
    };
    match rehydrated {
        Ok(Ok(session_id)) => Ok(session_id.to_string()),
        Ok(Err(error)) => Err(ResumeMiss::Rehydration(format!("{error:#}"))),
        Err(error) => Err(ResumeMiss::Rehydration(format!(
            "transcript rehydration task failed: {error}"
        ))),
    }
}

fn miss_from_fetch(error: AgentDriverError) -> ResumeMiss {
    match error {
        AgentDriverError::ConversationResumeStateMissing { .. } => ResumeMiss::NoTranscript,
        error => ResumeMiss::Rehydration(error.to_string()),
    }
}

/// The session the handshake should resume, given the rehydrated CLI session (`None` when the run
/// does not continue a restored conversation) and what the agent announced.
pub(super) fn session_to_resume(
    rehydrated: Option<Result<String, ResumeMiss>>,
    capabilities: &SessionCapabilities,
) -> Option<Result<String, ResumeMiss>> {
    rehydrated.map(|rehydrated| {
        rehydrated.and_then(|session_id| {
            if capabilities.supports_resume() {
                Ok(session_id)
            } else {
                Err(ResumeMiss::AgentCannotResume)
            }
        })
    })
}

/// Opens the run's ACP session: resumes `to_resume` (from [`session_to_resume`]) when there is
/// one, and otherwise, or when the agent refuses it, starts a new session.
pub(super) async fn open_session(
    connection: &AcpConnection,
    to_resume: Option<Result<String, ResumeMiss>>,
    cwd: String,
    mcp_servers: Vec<McpServer>,
) -> Result<(NewSessionResponse, SessionOrigin)> {
    let origin = match to_resume {
        None => SessionOrigin::New,
        Some(Err(miss)) => SessionOrigin::ResumeMissed(miss),
        Some(Ok(session_id)) => {
            let resumed = connection
                .request::<_, Option<ResumeSessionResponse>>(
                    protocol::METHOD_SESSION_RESUME,
                    ResumeSessionParams {
                        session_id: session_id.clone(),
                        cwd: cwd.clone(),
                        mcp_servers: mcp_servers.clone(),
                    },
                )
                .await;
            match resumed {
                Ok(resumed) => {
                    let session = resumed.unwrap_or_default().into_session(session_id);
                    return Ok((session, SessionOrigin::Resumed));
                }
                Err(error) => {
                    SessionOrigin::ResumeMissed(ResumeMiss::Rejected(format!("{error:#}")))
                }
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
