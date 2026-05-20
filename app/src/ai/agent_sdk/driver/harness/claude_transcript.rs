//! Claude Code transcript layout + rehydration helpers.
//!
//! This module owns:
//! - [`ClaudeTranscriptEnvelope`] — the on-wire/on-GCS shape of a saved Claude session
//!   (main jsonl entries + subagent jsonl files + per-agent todo JSONs), plus reader/writer
//!   functions that interoperate with Claude's own `~/.claude` layout.
//! - [`ClaudeResumeInfo`] — everything the harness runner needs to resume an existing
//!   Claude conversation: the Warp server conversation id to reuse, the Claude session uuid
//!   to pass to `claude --resume`, and the decoded envelope to rehydrate onto disk.
//! - [`write_session_index_entry`] — best-effort update of `~/.claude/sessions-index.json`
//!   so Claude's `--resume <uuid>` lookup can find the freshly-rehydrated jsonl. Upstream
//!   versions vary in how they use this index (claude-code#33912, #39667, #5768); we write
//!   a conservative entry and log on failure.
//!
//! Split out from `claude_code.rs` so the `AIClient` transcript-fetch impl can deserialize
//! envelopes without pulling in the rest of the harness runner.
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use warp_core::safe_warn;

use crate::ai::agent::conversation::AIConversationId;

use super::json_utils::entries_to_jsonl;

/// JSON envelope sent to the server representing a complete Claude Code session.
///
/// Bundles the main session transcript, any subagent transcripts, and
/// per-agent TODO lists assembled from the Claude state directory.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ClaudeTranscriptEnvelope {
    /// The directory that the Claude Code session started in.
    pub(crate) cwd: PathBuf,
    /// Unique session identifier.
    pub(crate) uuid: Uuid,
    /// Claude Code version, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) claude_version: Option<String>,
    /// List of messages in the main agent conversation.
    pub(crate) entries: Vec<Value>,
    /// Messages in each subagent conversation, keyed by the agent filename (e.g. `"agent-aac0b7f3db6bccfaf"`).
    pub(crate) subagents: HashMap<String, Vec<Value>>,
    /// TODO lists for each agent, keyed on the session and agent (e.g. `"<session_uuid>-agent-<agent_id>"`).
    pub(crate) todos: HashMap<String, Value>,
}

/// Everything needed to resume an existing Claude conversation.
///
/// Populated from a `--conversation` id after the client fetches the stored envelope from
/// the server. Passed into `ClaudeHarnessRunner::new` so the runner reuses the existing
/// session and server conversation ids instead of minting fresh ones.
#[derive(Debug)]
pub(crate) struct ClaudeResumeInfo {
    /// The Warp server-side conversation id. The runner stores this instead of calling
    /// `create_external_conversation` so subsequent transcript/block-snapshot uploads overwrite
    /// the same GCS objects.
    pub(crate) conversation_id: AIConversationId,
    /// The Claude session uuid to pass to `claude --resume`. Matches `envelope.uuid`.
    pub(crate) session_id: Uuid,
    /// Envelope from the server. Its `cwd` field is rewritten to the current run's working
    /// directory before being written to disk, so `claude --resume <uuid>` finds the jsonl under
    /// `~/.claude/projects/<encoded(new_cwd)>/`.
    pub(crate) envelope: ClaudeTranscriptEnvelope,
}

/// Encode a filesystem path as a Claude config directory name, matching the
/// Claude CLI convention of replacing every `/` with `-`.
///
/// Example: `/Users/ben/src/foo` → `-Users-ben-src-foo`
pub(crate) fn encode_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy().replace(['/', '.'], "-")
}

/// Resolve the Claude config directory.
///
/// Reads `$CLAUDE_CONFIG_DIR` if set, otherwise falls back to `~/.claude`.
//
/// TODO(REMOTE-1209): Use the transcript path reported by our hook.
pub(crate) fn claude_config_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    home_dir_for_claude_config()
        .map(|h| h.join(".claude"))
        .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))
}

/// In tests on Windows, `dirs::home_dir()` ignores `HOME`, so we check it
/// manually so that tests can override the home directory.
pub(super) fn home_dir_for_claude_config() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    dirs::home_dir()
}

/// Assemble a [`ClaudeTranscriptEnvelope`] from the Claude config directory.
///
/// Reads:
/// - `<config_root>/projects/<encoded_cwd>/<session_uuid>.jsonl` - main transcript
/// - `<config_root>/projects/<encoded_cwd>/<session_uuid>/subagents/*.jsonl` - subagents
/// - `<config_root>/todos/<session_uuid>-agent-*.json` - per-agent todo lists
///
/// If the main JSONL does not exist yet (e.g. during an early periodic save)
/// the envelope is returned with an empty `entries` list rather than an error.
pub(crate) fn read_envelope(
    session_uuid: Uuid,
    cwd: &Path,
    config_root: &Path,
) -> Result<ClaudeTranscriptEnvelope> {
    let encoded = encode_cwd(cwd);
    let projects_dir = config_root.join("projects").join(&encoded);

    // Main session transcript.
    let session_file = projects_dir.join(format!("{session_uuid}.jsonl"));
    let entries = read_jsonl(&session_file)?;

    // Subagents are stored in a directory named after the session UUID.
    let mut subagents: HashMap<String, Vec<Value>> = HashMap::new();
    let subagents_dir = projects_dir
        .join(session_uuid.to_string())
        .join("subagents");
    if subagents_dir.is_dir() {
        for entry in std::fs::read_dir(&subagents_dir)
            .with_context(|| format!("Failed to read subagents dir {}", subagents_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            subagents.insert(stem.to_owned(), read_jsonl(&path)?);
        }
    }

    // Per-agent todo lists.
    let mut todos: HashMap<String, Value> = HashMap::new();
    let todos_dir = config_root.join("todos");
    let todos_prefix = format!("{session_uuid}-agent-");
    if todos_dir.is_dir() {
        for entry in std::fs::read_dir(&todos_dir)
            .with_context(|| format!("Failed to read todos dir {}", todos_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !stem.starts_with(&todos_prefix) {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str(&content) {
                    Ok(value) => {
                        todos.insert(stem.to_owned(), value);
                    }
                    Err(e) => log::warn!("Failed to parse todos file {}: {e}", path.display()),
                },
                Err(e) => log::warn!("Failed to read todos file {}: {e}", path.display()),
            }
        }
    }

    Ok(ClaudeTranscriptEnvelope {
        cwd: cwd.to_path_buf(),
        uuid: session_uuid,
        claude_version: None,
        entries,
        subagents,
        todos,
    })
}

/// Write a [`ClaudeTranscriptEnvelope`] back to disk using the same layout
/// that Claude Code uses.
///
/// Creates:
/// - `<config_root>/projects/<encoded_cwd>/<uuid>.jsonl` - main transcript
/// - `<config_root>/projects/<encoded_cwd>/<uuid>/subagents/<stem>.jsonl` - subagents
/// - `<config_root>/todos/<stem>.json` - per-agent todo lists
pub(crate) fn write_envelope(
    envelope: &ClaudeTranscriptEnvelope,
    config_root: &Path,
) -> Result<()> {
    let encoded = encode_cwd(&envelope.cwd);
    let projects_dir = config_root.join("projects").join(&encoded);
    std::fs::create_dir_all(&projects_dir)
        .with_context(|| format!("Failed to create {}", projects_dir.display()))?;

    // Main session JSONL.
    let session_file = projects_dir.join(format!("{}.jsonl", envelope.uuid));
    std::fs::write(&session_file, entries_to_jsonl(&envelope.entries)?)
        .with_context(|| format!("Failed to write {}", session_file.display()))?;

    // Subagent JSONLs.
    if !envelope.subagents.is_empty() {
        let subagents_dir = projects_dir
            .join(envelope.uuid.to_string())
            .join("subagents");
        std::fs::create_dir_all(&subagents_dir)
            .with_context(|| format!("Failed to create {}", subagents_dir.display()))?;
        for (stem, entries) in &envelope.subagents {
            let path = subagents_dir.join(format!("{stem}.jsonl"));
            std::fs::write(&path, entries_to_jsonl(entries)?)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }
    }

    // Per-agent todo lists.
    if !envelope.todos.is_empty() {
        let todos_dir = config_root.join("todos");
        std::fs::create_dir_all(&todos_dir)
            .with_context(|| format!("Failed to create {}", todos_dir.display()))?;
        for (stem, value) in &envelope.todos {
            let path = todos_dir.join(format!("{stem}.json"));
            std::fs::write(&path, serde_json::to_vec(value)?)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }
    }

    Ok(())
}

/// Filename of Claude's global session index.
const SESSIONS_INDEX_FILENAME: &str = "sessions-index.json";

/// Upsert an entry for `session_uuid` into `<config_root>/sessions-index.json` so Claude's
/// `claude --resume <uuid>` lookup can find the rehydrated jsonl.
///
/// Upstream Claude versions vary in how the index is keyed and what fields they read; this
/// writer uses a conservative session-uuid-keyed schema (session id, cwd, jsonl path) that
/// mirrors the fragments documented in claude-code#33912 / #39667 / #5768. Unknown fields are
/// preserved on existing entries, and we never remove other entries.
///
/// Best-effort: callers should log a warning on failure rather than aborting the run — if the
/// index is missing or wrong, `--resume` simply falls back to "No conversation found" and the
/// resumed run surfaces the expected resume-failure error.
pub(crate) fn write_session_index_entry(
    session_uuid: Uuid,
    cwd: &Path,
    config_root: &Path,
) -> Result<()> {
    let index_path = config_root.join(SESSIONS_INDEX_FILENAME);

    // Read the existing index if present. Missing or malformed files are treated as empty —
    // we'd rather clobber an unparsable file than fail the whole resume.
    let mut index: serde_json::Map<String, Value> = match std::fs::read_to_string(&index_path) {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(Value::Object(map)) => map,
            Ok(_) => {
                safe_warn!(
                    safe: ("sessions-index.json is not a JSON object; overwriting"),
                    full: ("sessions-index.json at {} is not a JSON object; overwriting", index_path.display())
                );
                serde_json::Map::new()
            }
            Err(e) => {
                safe_warn!(
                    safe: ("Failed to parse sessions-index.json; overwriting"),
                    full: ("Failed to parse sessions-index.json at {}: {e}; overwriting", index_path.display())
                );
                serde_json::Map::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::Map::new(),
        Err(e) => {
            return Err(
                anyhow::Error::from(e).context(format!("Failed to read {}", index_path.display()))
            );
        }
    };

    let encoded = encode_cwd(cwd);
    let transcript_path = format!("projects/{encoded}/{session_uuid}.jsonl");
    let entry = serde_json::json!({
        "sessionId": session_uuid.to_string(),
        "cwd": cwd.to_string_lossy(),
        "projectPath": encoded,
        "transcriptPath": transcript_path,
    });
    index.insert(session_uuid.to_string(), entry);

    if let Some(parent) = index_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    std::fs::write(
        &index_path,
        serde_json::to_vec_pretty(&Value::Object(index))
            .context("Failed to serialize sessions-index.json")?,
    )
    .with_context(|| format!("Failed to write {}", index_path.display()))?;
    Ok(())
}

/// Read a JSONL file, returning one parsed [`Value`] per non-blank line.
///
/// Lines that fail to parse as JSON are skipped with a warning rather than
/// causing the entire read to fail. A missing file returns an empty [`Vec`].
pub(crate) fn read_jsonl(path: &Path) -> Result<Vec<Value>> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(
                anyhow::Error::from(e).context(format!("Failed to open {}", path.display()))
            );
        }
    };
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let line = line.with_context(|| format!("Failed to read line from {}", path.display()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str(trimmed) {
            Ok(value) => entries.push(value),
            Err(e) => {
                safe_warn!(
                    safe: ("Skipping malformed JSONL entry"),
                    full: ("Skipping malformed JSONL entry in {}: {e}", path.display())
                );
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "claude_transcript_tests.rs"]
mod tests;
