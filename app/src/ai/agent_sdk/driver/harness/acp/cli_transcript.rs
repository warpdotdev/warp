//! The CLI behind an ACP adapter writes the same session transcript it writes when run in a
//! terminal, keyed by the ACP session id: codex-acp returns the Codex thread id from
//! `session/new`, and claude-agent-acp passes its session id to the Agent SDK as the Claude
//! session id. The terminal runners' capture code therefore applies unchanged.
//!
//! One gap: when claude-agent-acp restarts a session to clear its context after a plan is
//! approved, the SDK continues under a fresh id that the adapter never reports. Captures keep
//! reading the transcript under the ACP session id, which stops at the restart.
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use uuid::Uuid;
use warp_cli::agent::Harness;

use super::super::claude_transcript::{claude_config_dir, session_transcript_path};
use super::super::codex_transcript::{codex_sessions_root, find_session_file};
use super::super::transcript_persistence::{CapturedTranscript, UploadedTranscriptUsage};
use super::super::usage_reporting::CaptureIdentity;
use super::super::{claude_code, codex};

/// The CLI transcript format an ACP harness produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CliTranscriptKind {
    Codex,
    Claude,
}

impl CliTranscriptKind {
    pub(super) fn for_harness(harness: Harness) -> Option<Self> {
        match harness {
            Harness::Codex => Some(Self::Codex),
            Harness::Claude => Some(Self::Claude),
            Harness::Gemini | Harness::Oz | Harness::OpenCode | Harness::Unknown => None,
        }
    }

    /// The directory the CLI keeps its transcripts under, from the same environment the adapter
    /// inherits.
    pub(super) fn root(self) -> Result<PathBuf> {
        match self {
            Self::Codex => codex_sessions_root(),
            Self::Claude => claude_config_dir(),
        }
    }

    /// The transcript file, once the CLI has written it. `root` is [`Self::root`].
    pub(super) fn locate(self, root: &Path, cwd: &Path, session_id: Uuid) -> Option<PathBuf> {
        match self {
            Self::Codex => find_session_file(root, session_id),
            Self::Claude => {
                Some(session_transcript_path(root, cwd, session_id)).filter(|path| path.exists())
            }
        }
    }

    /// Captures the transcript at `path` exactly as the terminal runner for this CLI does.
    pub(super) fn capture(
        self,
        session_id: Uuid,
        path: &Path,
        root: &Path,
        cwd: &Path,
        is_final: bool,
        identity: Option<CaptureIdentity>,
    ) -> Result<CapturedTranscript> {
        match self {
            Self::Codex => {
                codex::capture_transcript_with_usage(session_id, path, is_final, identity)
            }
            Self::Claude => claude_code::capture_transcript_with_usage(
                session_id, cwd, root, None, is_final, identity,
            ),
        }
    }

    /// The save result when the transcript does not exist yet. The CLI writes it only after the
    /// first prompt, so its absence during the run is expected; at the final save each CLI
    /// keeps its terminal runner's behavior.
    pub(super) fn missing_transcript(self, is_final: bool) -> Result<UploadedTranscriptUsage> {
        match (self, is_final) {
            (Self::Codex | Self::Claude, false) => {
                log::debug!("{self:?} transcript for the ACP session is not available yet");
                Ok(UploadedTranscriptUsage::empty())
            }
            (Self::Codex, true) => {
                log::warn!(
                    "Codex rollout still unavailable at final save; transcript was never uploaded"
                );
                Ok(UploadedTranscriptUsage::empty())
            }
            (Self::Claude, true) => Err(anyhow!(
                "Claude Code transcript does not exist after harness termination"
            )),
        }
    }
}

#[cfg(test)]
#[path = "cli_transcript_tests.rs"]
mod tests;
