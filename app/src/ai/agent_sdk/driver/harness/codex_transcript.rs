//! Codex session transcript envelope + rehydration helpers.
//!
//! Owns:
//! - [`CodexTranscriptEnvelope`] — the on-wire/on-GCS shape of a saved Codex rollout
//!   (parsed JSONL entries plus session-level metadata). Reader/writer functions
//!   interoperate with Codex's own `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`
//!   layout (codex `rollout/src/recorder.rs`).
//! - [`CodexResumeInfo`] — everything the harness runner needs to resume an existing
//!   Codex conversation: the Warp server conversation id to reuse, the codex session
//!   uuid (`ThreadId`) to pass to `codex resume`, and the decoded envelope to rehydrate
//!   onto disk.
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::json_utils::entries_to_jsonl;

use crate::ai::agent::conversation::AIConversationId;

/// Env var codex honors to override `~/.codex` (see codex `core/src/config/mod.rs`).
const CODEX_HOME_ENV: &str = "CODEX_HOME";
const CODEX_HOME_DIRNAME: &str = ".codex";
/// Subdirectory under `$CODEX_HOME` where rollouts live.
const CODEX_SESSIONS_SUBDIR: &str = "sessions";

/// JSON envelope sent to the server representing a complete Codex session.
///
/// The transcript is the parsed JSONL content of the rollout file; codex's resume
/// path re-reads this JSONL line by line.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CodexTranscriptEnvelope {
    /// The directory the codex session started in (recovered from the `SessionMeta` line).
    pub(crate) cwd: PathBuf,
    /// Codex session/thread UUID. Matches the trailing `-<uuid>` in the rollout filename.
    pub(crate) session_id: Uuid,
    /// `cli_version` from `SessionMeta`, surfaced separately for the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) codex_version: Option<String>,
    /// Timestamp from the `SessionMeta` line, used to derive the YYYY/MM/DD directory
    /// path when writing the rollout file back to disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session_start_timestamp: Option<DateTime<Utc>>,
    /// Parsed JSONL entries.
    pub(crate) entries: Vec<Value>,
}

impl CodexTranscriptEnvelope {
    pub(crate) fn new(session_id: Uuid, meta: CodexSessionMetadata, entries: Vec<Value>) -> Self {
        Self {
            cwd: meta.cwd,
            session_id,
            codex_version: meta.codex_version,
            session_start_timestamp: meta.session_start_timestamp,
            entries,
        }
    }
}

/// Session-level metadata pulled from the rollout's `SessionMeta` line.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CodexSessionMetadata {
    pub(crate) cwd: PathBuf,
    pub(crate) codex_version: Option<String>,
    pub(crate) session_start_timestamp: Option<DateTime<Utc>>,
}

/// Everything needed to resume an existing Codex conversation.
///
/// Built from a `--conversation` id after the client fetches the stored envelope from
/// the server. Passed into `CodexHarnessRunner::new` so the runner reuses the existing
/// session and server conversation ids instead of minting fresh ones.
#[derive(Debug)]
pub(crate) struct CodexResumeInfo {
    /// Warp server-side conversation id. Reused so subsequent transcript/block-snapshot
    /// uploads overwrite the same GCS objects.
    pub(crate) conversation_id: AIConversationId,
    /// Codex session uuid passed to `codex resume <session_id>`. Matches `envelope.session_id`.
    pub(crate) session_id: Uuid,
    /// Envelope fetched from the server, written back to disk before launching codex.
    pub(crate) envelope: CodexTranscriptEnvelope,
}

/// Resolve the codex sessions root, honoring `$CODEX_HOME` then falling back to `~/.codex`.
pub(crate) fn codex_sessions_root() -> anyhow::Result<PathBuf> {
    let home = if let Ok(dir) = std::env::var(CODEX_HOME_ENV) {
        PathBuf::from(dir)
    } else {
        dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?
            .join(CODEX_HOME_DIRNAME)
    };
    Ok(home.join(CODEX_SESSIONS_SUBDIR))
}

/// Walk `<sessions_root>/YYYY/MM/DD/` looking for a `rollout-*-<session_id>.jsonl`.
///
/// Returns `None` if `sessions_root` doesn't exist yet or no matching file is found.
pub(crate) fn find_session_file(sessions_root: &Path, session_id: Uuid) -> Option<PathBuf> {
    if !sessions_root.exists() {
        return None;
    }
    let suffix = format!("-{session_id}.jsonl");
    for year_dir in read_subdirs(sessions_root) {
        for month_dir in read_subdirs(&year_dir) {
            for day_dir in read_subdirs(&month_dir) {
                let entries = match fs::read_dir(&day_dir) {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    if name.starts_with("rollout-") && name.ends_with(&suffix) {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

fn read_subdirs(parent: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            entry.file_type().ok()?.is_dir().then(|| entry.path())
        })
}

/// Pull `cwd` and `cli_version` out of the first JSONL line if it's a `SessionMeta`.
pub(crate) fn parse_session_meta(first: Option<&Value>) -> Option<CodexSessionMetadata> {
    let entry = first?;
    if entry.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
        return None;
    }
    let payload = entry.get("payload")?;
    let cwd = PathBuf::from(payload.get("cwd").and_then(|v| v.as_str())?);
    let codex_version = payload
        .get("cli_version")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let session_start_timestamp = payload
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    Some(CodexSessionMetadata {
        cwd,
        codex_version,
        session_start_timestamp,
    })
}

/// Write `envelope` back under `<sessions_root>/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`.
///
/// YYYY/MM/DD and `<ts>` come from `envelope.session_start_timestamp`. Falls back to
/// today's UTC date if absent — codex's lookup is by UUID so the precise path doesn't
/// matter for resume to work.
pub(crate) fn write_envelope(
    envelope: &CodexTranscriptEnvelope,
    sessions_root: &Path,
) -> Result<PathBuf> {
    let timestamp = envelope.session_start_timestamp.unwrap_or_else(Utc::now);
    let day_dir = sessions_root
        .join(format!("{:04}", timestamp.year()))
        .join(format!("{:02}", timestamp.month()))
        .join(format!("{:02}", timestamp.day()));
    fs::create_dir_all(&day_dir)
        .with_context(|| format!("Failed to create {}", day_dir.display()))?;
    // Codex's filename format: `[year]-[month]-[day]T[hour]-[minute]-[second]`
    // (codex `rollout/src/recorder.rs::precompute_log_file_info`).
    let date_str = timestamp.format("%Y-%m-%dT%H-%M-%S").to_string();
    let file_path = day_dir.join(format!(
        "rollout-{date_str}-{session_id}.jsonl",
        session_id = envelope.session_id
    ));
    fs::write(&file_path, entries_to_jsonl(&envelope.entries)?)
        .with_context(|| format!("Failed to write {}", file_path.display()))?;
    Ok(file_path)
}

#[cfg(test)]
#[path = "codex_transcript_tests.rs"]
mod tests;
